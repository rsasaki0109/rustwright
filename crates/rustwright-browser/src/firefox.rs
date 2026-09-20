//! Firefox discovery, launch and process lifecycle for WebDriver BiDi.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use crate::discovery::{find_firefox, firefox_candidates};
use crate::error::{BrowserError, BrowserResult};
use crate::launch::{pick_free_port, unique_suffix, DEFAULT_STARTUP_TIMEOUT};

const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Describes which Firefox to launch and with what options.
///
/// Firefox is driven over **WebDriver BiDi**; the launcher starts the browser
/// with its remote agent and exposes the BiDi WebSocket URL.
#[derive(Debug, Clone)]
pub struct Firefox {
    executable: Option<PathBuf>,
    headless: bool,
    profile: Option<PathBuf>,
    args: Vec<String>,
    envs: Vec<(String, String)>,
    startup_timeout: Duration,
    capture_stderr: bool,
}

impl Firefox {
    /// Use the first Firefox installed on this machine.
    pub fn installed() -> Self {
        Self {
            executable: None,
            headless: false,
            profile: None,
            args: Vec::new(),
            envs: Vec::new(),
            startup_timeout: DEFAULT_STARTUP_TIMEOUT,
            capture_stderr: true,
        }
    }

    /// Use a specific Firefox binary.
    pub fn at(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: Some(executable.into()),
            ..Self::installed()
        }
    }

    /// Run headless (`true`) or with a visible window (`false`).
    pub fn headless(mut self, headless: bool) -> Self {
        self.headless = headless;
        self
    }

    /// Use a specific profile directory.
    ///
    /// Note: sandboxed Firefox builds (for example the Snap) cannot read
    /// arbitrary profile paths and will exit during startup.
    pub fn profile(mut self, path: impl Into<PathBuf>) -> Self {
        self.profile = Some(path.into());
        self
    }

    /// Append a command-line flag.
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.args.push(arg.into());
        self
    }

    /// Set an environment variable for the Firefox process.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.envs.push((key.into(), value.into()));
        self
    }

    /// Override how long to wait for the BiDi endpoint on launch.
    pub fn startup_timeout(mut self, timeout: Duration) -> Self {
        self.startup_timeout = timeout;
        self
    }

    /// Capture Firefox's stderr to a log file (default: `true`).
    pub fn capture_stderr(mut self, capture: bool) -> Self {
        self.capture_stderr = capture;
        self
    }

    /// Whether headless mode is enabled.
    pub fn is_headless(&self) -> bool {
        self.headless
    }

    /// The configured startup timeout.
    pub fn startup_timeout_value(&self) -> Duration {
        self.startup_timeout
    }

    /// Whether stderr capture is enabled.
    pub fn captures_stderr(&self) -> bool {
        self.capture_stderr
    }

    /// Extra command-line flags.
    pub fn extra_args(&self) -> &[String] {
        &self.args
    }

    /// Environment overrides.
    pub fn environment(&self) -> &[(String, String)] {
        &self.envs
    }

    /// The configured profile, if any.
    pub fn profile_path(&self) -> Option<&Path> {
        self.profile.as_deref()
    }

    /// Whether the resolved Firefox build is sandboxed (for example a Snap).
    ///
    /// Sandboxed builds ignore custom profiles and cannot read `file://` URLs
    /// outside their sandbox, so launch falls back to the default profile.
    pub fn is_sandboxed(&self) -> bool {
        self.executable_path()
            .map(|path| is_sandboxed_launcher(&path))
            .unwrap_or(false)
    }

    /// Resolve the Firefox executable to an existing path.
    pub fn executable_path(&self) -> BrowserResult<PathBuf> {
        if let Some(explicit) = &self.executable {
            return if explicit.is_file() {
                Ok(explicit.clone())
            } else {
                Err(BrowserError::ExecutableNotFound(explicit.clone()))
            };
        }
        if let Some(configured) = std::env::var_os(crate::discovery::FIREFOX_PATH_ENV) {
            let path = PathBuf::from(configured);
            if path.is_file() {
                return Ok(path);
            }
        }
        find_firefox().ok_or_else(|| BrowserError::NotFound {
            search_paths: firefox_candidates(),
        })
    }
}

/// A Firefox process launched by Rustwright, exposing its BiDi endpoint.
pub struct LaunchedFirefox {
    child: Child,
    port: u16,
    ws_url: String,
    executable: PathBuf,
    launch_args: Vec<String>,
    profile: Option<PathBuf>,
    stderr_log: Option<PathBuf>,
    leaked: bool,
}

impl std::fmt::Debug for LaunchedFirefox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LaunchedFirefox")
            .field("pid", &self.child.id())
            .field("executable", &self.executable)
            .field("ws_url", &self.ws_url)
            .finish()
    }
}

impl LaunchedFirefox {
    /// Launch Firefox and wait until its WebDriver BiDi endpoint is ready.
    pub async fn launch(firefox: &Firefox) -> BrowserResult<Self> {
        let executable = firefox.executable_path()?;
        let port = pick_free_port()?;
        let stderr_log = if firefox.captures_stderr() {
            Some(std::env::temp_dir().join(format!("rustwright-firefox-{}.log", unique_suffix())))
        } else {
            None
        };

        // Sandboxed builds (for example the Snap) cannot read arbitrary profile
        // paths, so fall back to the browser's own default profile instead of
        // failing to start.
        let sandboxed = is_sandboxed_launcher(&executable);
        let effective_profile = if sandboxed {
            if firefox.profile_path().is_some() {
                tracing::warn!(
                    "Firefox at {} looks sandboxed (snap); ignoring the custom profile and \
                     using the browser's default profile",
                    executable.display()
                );
            }
            None
        } else {
            firefox.profile_path()
        };

        let mut args = Vec::new();
        if firefox.is_headless() {
            args.push("--headless".to_string());
        }
        args.push("--no-remote".to_string());
        if let Some(profile) = effective_profile {
            std::fs::create_dir_all(profile).map_err(|source| BrowserError::ProfileDir {
                path: profile.to_path_buf(),
                source,
            })?;
            args.push("--profile".to_string());
            args.push(profile.display().to_string());
        }
        args.push(format!("--remote-debugging-port={port}"));
        args.extend(firefox.extra_args().iter().cloned());
        args.push("about:blank".to_string());

        tracing::debug!(?executable, ?args, "launching firefox");
        let mut command = Command::new(&executable);
        command
            .args(&args)
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
        for (key, value) in firefox.environment() {
            command.env(key, value);
        }

        let mut child = command.spawn().map_err(|source| BrowserError::Launch {
            executable: executable.clone(),
            source,
        })?;

        wait_for_port(
            &mut child,
            port,
            firefox.startup_timeout_value(),
            stderr_log.as_deref(),
        )
        .await?;

        Ok(Self {
            child,
            port,
            ws_url: format!("ws://127.0.0.1:{port}/session"),
            executable,
            launch_args: args,
            profile: effective_profile.map(Path::to_path_buf),
            stderr_log,
            leaked: false,
        })
    }

    /// The WebDriver BiDi WebSocket URL.
    pub fn ws_url(&self) -> &str {
        &self.ws_url
    }

    /// The chosen remote-debugging port.
    pub fn port(&self) -> u16 {
        self.port
    }

    /// The Firefox executable that was launched.
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    /// The exact command-line flags used.
    pub fn launch_args(&self) -> &[String] {
        &self.launch_args
    }

    /// The profile directory in use, if Rustwright supplied one.
    pub fn profile(&self) -> Option<&Path> {
        self.profile.as_deref()
    }

    /// Path to the captured Firefox stderr log, if enabled.
    pub fn stderr_log(&self) -> Option<&Path> {
        self.stderr_log.as_deref()
    }

    /// The operating system process id.
    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// Whether the process is still running.
    pub fn is_running(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// Terminate the browser process.
    pub fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    /// Prevent cleanup on drop, handing process ownership to the caller.
    pub fn leak(mut self) {
        self.leaked = true;
    }
}

impl Drop for LaunchedFirefox {
    fn drop(&mut self) {
        if self.leaked {
            return;
        }
        self.kill();
    }
}

fn is_sandboxed_launcher(executable: &Path) -> bool {
    if std::env::var_os("SNAP").is_some() {
        return true;
    }
    if executable.starts_with("/snap") {
        return true;
    }
    std::fs::read_to_string(executable)
        .map(|contents| {
            contents.contains("/snap/") || contents.to_ascii_lowercase().contains("snap")
        })
        .unwrap_or(false)
}

async fn wait_for_port(
    child: &mut Child,
    port: u16,
    timeout: Duration,
    stderr_log: Option<&Path>,
) -> BrowserResult<()> {
    let deadline = Instant::now() + timeout;
    loop {
        let connect = tokio::time::timeout(
            Duration::from_millis(500),
            tokio::net::TcpStream::connect(("127.0.0.1", port)),
        )
        .await;
        if matches!(connect, Ok(Ok(_))) {
            return Ok(());
        }
        if let Ok(Some(status)) = child.try_wait() {
            return Err(BrowserError::EarlyExit {
                status: status.to_string(),
                log_path: stderr_log
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from("<stderr capture disabled>")),
            });
        }
        if Instant::now() >= deadline {
            return Err(BrowserError::StartupTimeout {
                timeout,
                log_path: stderr_log
                    .map(Path::to_path_buf)
                    .unwrap_or_else(|| PathBuf::from("<stderr capture disabled>")),
            });
        }
        tokio::time::sleep(STARTUP_POLL_INTERVAL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::is_sandboxed_launcher;

    #[test]
    fn detects_snap_wrapper_scripts() {
        let dir =
            std::env::temp_dir().join(format!("rw-firefox-{}", crate::launch::unique_suffix()));
        std::fs::create_dir_all(&dir).expect("temp dir");

        let wrapper = dir.join("firefox");
        std::fs::write(
            &wrapper,
            "#!/bin/sh\n# firefox snap wrapper\nif ! [ -x /snap/bin/firefox ]; then exit 1; fi\n",
        )
        .expect("write wrapper");
        assert!(is_sandboxed_launcher(&wrapper));

        let normal = dir.join("normal-firefox");
        std::fs::write(&normal, "a normal build without the marker").expect("write normal");
        assert!(!is_sandboxed_launcher(&normal));

        let _ = std::fs::remove_dir_all(&dir);
    }
}
