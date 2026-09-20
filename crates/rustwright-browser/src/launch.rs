//! Launching a browser process and waiting for its DevTools endpoint.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rustwright_cdp::{BrowserVersion, HttpEndpoint};

use crate::chrome::{Chrome, Profile};
use crate::error::{BrowserError, BrowserResult};

/// Default time to wait for a launched browser to expose its DevTools endpoint.
pub const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// A browser process launched by Rustwright.
///
/// The process is killed and its ephemeral profile removed when this value is
/// dropped, unless taken over with [`LaunchedBrowser::leak`].
pub struct LaunchedBrowser {
    child: Child,
    user_data_dir: PathBuf,
    ephemeral: bool,
    executable: PathBuf,
    launch_args: Vec<String>,
    endpoint: HttpEndpoint,
    ws_url: String,
    stderr_log: Option<PathBuf>,
    leaked: bool,
}

impl std::fmt::Debug for LaunchedBrowser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchedBrowser")
            .field("pid", &self.child.id())
            .field("executable", &self.executable)
            .field("endpoint", &self.endpoint.base_url())
            .field("ephemeral", &self.ephemeral)
            .finish()
    }
}

impl LaunchedBrowser {
    /// Launch `chrome` and wait until its DevTools endpoint is ready.
    pub async fn launch(chrome: &Chrome) -> BrowserResult<Self> {
        let executable = chrome.executable_path()?;
        let (user_data_dir, ephemeral, active_port_dir) = prepare_profile(chrome)?;
        let stderr_log = if chrome.captures_stderr() {
            Some(user_data_dir.join("rustwright-chrome.log"))
        } else {
            None
        };

        let use_active_port = active_port_dir.is_some();
        let fixed_port = if use_active_port {
            0
        } else {
            pick_free_port()?
        };
        let launch_args = build_args(chrome, &user_data_dir, fixed_port, use_active_port);

        // Remove a stale `DevToolsActivePort` from a previous run so we never
        // connect to a dead port before this process writes its own.
        if let Some(dir) = &active_port_dir {
            let _ = std::fs::remove_file(dir.join("DevToolsActivePort"));
        }

        tracing::debug!(?executable, ?launch_args, "launching browser");
        let mut command = Command::new(&executable);
        command
            .args(&launch_args)
            .stdin(Stdio::null())
            .stdout(Stdio::null());
        match &stderr_log {
            Some(path) => {
                let file =
                    std::fs::File::create(path).map_err(|source| BrowserError::ProfileDir {
                        path: path.clone(),
                        source,
                    })?;
                command.stderr(Stdio::from(file));
            }
            None => {
                command.stderr(Stdio::null());
            }
        }
        for (key, value) in chrome.environment() {
            command.env(key, value);
        }

        let mut child = command.spawn().map_err(|source| BrowserError::Launch {
            executable: executable.clone(),
            source,
        })?;

        let (endpoint, ws_url) = match &active_port_dir {
            Some(dir) => {
                wait_for_active_port(
                    &mut child,
                    dir,
                    &executable,
                    chrome.startup_timeout_value(),
                    &stderr_log,
                )
                .await?
            }
            None => {
                wait_for_http_endpoint(
                    &mut child,
                    fixed_port,
                    chrome.startup_timeout_value(),
                    &stderr_log,
                )
                .await?
            }
        };

        Ok(Self {
            child,
            user_data_dir,
            ephemeral,
            executable,
            launch_args,
            endpoint,
            ws_url,
            stderr_log,
            leaked: false,
        })
    }

    /// The DevTools HTTP endpoint, e.g. `http://127.0.0.1:9222`.
    pub fn endpoint(&self) -> &HttpEndpoint {
        &self.endpoint
    }

    /// The browser-level WebSocket debugger URL.
    pub fn ws_url(&self) -> &str {
        &self.ws_url
    }

    /// The browser executable that was launched.
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// The exact command-line flags used to launch the browser.
    pub fn launch_args(&self) -> &[String] {
        &self.launch_args
    }

    /// The user data directory in use.
    pub fn user_data_dir(&self) -> &Path {
        &self.user_data_dir
    }

    /// Whether the profile is temporary.
    pub fn is_ephemeral(&self) -> bool {
        self.ephemeral
    }

    /// Path to the captured browser stderr log, if enabled.
    pub fn stderr_log(&self) -> Option<&Path> {
        self.stderr_log.as_deref()
    }

    /// The operating system process id.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Whether the browser process is still running.
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Terminate the browser process.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Prevent cleanup on drop, handing process ownership to the caller.
    ///
    /// The process and its profile are left in place; the caller is responsible
    /// for terminating the browser (for example with [`Self::kill`]).
    pub fn leak(mut self) {
        self.leaked = true;
    }
}

impl Drop for LaunchedBrowser {
    fn drop(&mut self) {
        if self.leaked {
            return;
        }
        self.kill();
        if self.ephemeral {
            if let Err(error) = std::fs::remove_dir_all(&self.user_data_dir) {
                tracing::debug!(%error, dir = %self.user_data_dir.display(), "failed to remove ephemeral profile");
            }
        }
    }
}

fn prepare_profile(chrome: &Chrome) -> BrowserResult<(PathBuf, bool, Option<PathBuf>)> {
    match chrome.profile_mode() {
        Profile::Ephemeral => {
            let dir = std::env::temp_dir().join(format!("rustwright-{}", unique_suffix()));
            std::fs::create_dir_all(&dir).map_err(|source| BrowserError::ProfileDir {
                path: dir.clone(),
                source,
            })?;
            Ok((dir.clone(), true, Some(dir)))
        }
        Profile::Persistent(path) => {
            std::fs::create_dir_all(path).map_err(|source| BrowserError::ProfileDir {
                path: path.clone(),
                source,
            })?;
            Ok((path.clone(), false, Some(path.clone())))
        }
        Profile::Default => {
            let dir = std::env::temp_dir().join(format!("rustwright-{}", unique_suffix()));
            Ok((dir, false, None))
        }
    }
}

fn build_args(
    chrome: &Chrome,
    user_data_dir: &Path,
    port: u16,
    use_profile_arg: bool,
) -> Vec<String> {
    let mut args = vec![
        format!("--remote-debugging-port={port}"),
        "--no-first-run".to_string(),
        "--no-default-browser-check".to_string(),
    ];
    if use_profile_arg {
        args.push(format!("--user-data-dir={}", user_data_dir.display()));
    }
    if chrome.is_headless() {
        args.push("--headless=new".to_string());
        args.push("--disable-gpu".to_string());
        args.push("--hide-scrollbars".to_string());
        args.push("--mute-audio".to_string());
    }
    args.extend(chrome.extra_args().iter().cloned());
    args.push("about:blank".to_string());
    args
}

async fn wait_for_active_port(
    child: &mut Child,
    profile_dir: &Path,
    executable: &Path,
    timeout: Duration,
    stderr_log: &Option<PathBuf>,
) -> BrowserResult<(HttpEndpoint, String)> {
    let active_port = profile_dir.join("DevToolsActivePort");
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(contents) = std::fs::read_to_string(&active_port) {
            if let Some(parsed) = parse_active_port(&contents) {
                return Ok(parsed);
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(BrowserError::EarlyExit {
                status: status.to_string(),
                log_path: log_path(stderr_log),
            });
        }
        if Instant::now() >= deadline {
            let _ = executable;
            return Err(BrowserError::StartupTimeout {
                timeout,
                log_path: log_path(stderr_log),
            });
        }
        tokio::time::sleep(STARTUP_POLL_INTERVAL).await;
    }
}

async fn wait_for_http_endpoint(
    child: &mut Child,
    port: u16,
    timeout: Duration,
    stderr_log: &Option<PathBuf>,
) -> BrowserResult<(HttpEndpoint, String)> {
    let endpoint = HttpEndpoint {
        host: "127.0.0.1".to_string(),
        port,
    };
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(version) = BrowserVersion::fetch(&endpoint).await {
            if !version.web_socket_debugger_url.is_empty() {
                return Ok((endpoint, version.web_socket_debugger_url));
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(BrowserError::EarlyExit {
                status: status.to_string(),
                log_path: log_path(stderr_log),
            });
        }
        if Instant::now() >= deadline {
            return Err(BrowserError::StartupTimeout {
                timeout,
                log_path: log_path(stderr_log),
            });
        }
        tokio::time::sleep(STARTUP_POLL_INTERVAL).await;
    }
}

fn parse_active_port(contents: &str) -> Option<(HttpEndpoint, String)> {
    let mut lines = contents.lines();
    let port = lines.next()?.trim().parse::<u16>().ok()?;
    let ws_path = lines.next()?.trim();
    if !ws_path.starts_with('/') {
        return None;
    }
    let endpoint = HttpEndpoint {
        host: "127.0.0.1".to_string(),
        port,
    };
    let ws_url = format!("ws://127.0.0.1:{port}{ws_path}");
    Some((endpoint, ws_url))
}

fn log_path(stderr_log: &Option<PathBuf>) -> PathBuf {
    stderr_log
        .clone()
        .unwrap_or_else(|| PathBuf::from("<stderr capture disabled>"))
}

pub(crate) fn pick_free_port() -> BrowserResult<u16> {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    let port = listener.local_addr()?.port();
    drop(listener);
    Ok(port)
}

pub(crate) fn unique_suffix() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}-{nanos}-{counter}", std::process::id())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_devtools_active_port_file() {
        let contents = "54321\n/devtools/browser/abc-def\n";
        let (endpoint, ws_url) = parse_active_port(contents).expect("parsed");
        assert_eq!(endpoint.port, 54321);
        assert_eq!(endpoint.host, "127.0.0.1");
        assert_eq!(ws_url, "ws://127.0.0.1:54321/devtools/browser/abc-def");
    }

    #[test]
    fn rejects_malformed_active_port_file() {
        assert!(parse_active_port("not-a-port\n/devtools/browser/x").is_none());
        assert!(parse_active_port("1234\nno-leading-slash").is_none());
        assert!(parse_active_port("").is_none());
    }

    #[test]
    fn build_args_includes_profile_and_headless_flags() {
        let chrome = Chrome::installed().headless(true).arg("--custom-flag");
        let args = build_args(&chrome, Path::new("/tmp/profile"), 0, true);
        assert!(args.iter().any(|arg| arg == "--remote-debugging-port=0"));
        assert!(args.iter().any(|arg| arg == "--user-data-dir=/tmp/profile"));
        assert!(args.iter().any(|arg| arg == "--headless=new"));
        assert!(args.iter().any(|arg| arg == "--custom-flag"));
        assert_eq!(args.last().map(String::as_str), Some("about:blank"));
    }
}
