//! Launching a browser process and waiting for its DevTools endpoint.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use rustwright_cdp::{BrowserVersion, HttpEndpoint};

use crate::chrome::{Chrome, Profile};
use crate::error::{BrowserError, BrowserResult};

/// Default time to wait for a launched browser to expose its DevTools endpoint.
pub const DEFAULT_STARTUP_TIMEOUT: Duration = Duration::from_secs(30);

const STARTUP_POLL_INTERVAL: Duration = Duration::from_millis(10);

/// Owns startup resources before readiness can suspend or fail.
pub(crate) struct StartupGuard {
    child: Option<Child>,
    ephemeral_profile: Option<PathBuf>,
}

impl StartupGuard {
    pub(crate) fn new(ephemeral_profile: Option<PathBuf>) -> Self {
        Self {
            child: None,
            ephemeral_profile,
        }
    }

    pub(crate) fn own_child(&mut self, child: Child) {
        self.child = Some(child);
    }

    pub(crate) fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("startup owns its child")
    }

    pub(crate) fn ready(mut self) -> Child {
        self.ephemeral_profile = None;
        self.child.take().expect("startup owns its child")
    }
}

impl Drop for StartupGuard {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let stopped = match child.try_wait() {
                Ok(Some(_)) => true,
                _ => match child.kill() {
                    Ok(()) => match child.wait() {
                        Ok(_) => true,
                        Err(error) => {
                            tracing::warn!(%error, pid = child.id(), "failed to reap startup child");
                            false
                        }
                    },
                    Err(error) => {
                        // A child can exit between try_wait and kill.
                        let stopped = matches!(child.try_wait(), Ok(Some(_)));
                        if !stopped {
                            tracing::warn!(%error, pid = child.id(), "failed to terminate startup child");
                        }
                        stopped
                    }
                },
            };
            if !stopped {
                // Never remove a profile while its process may still be using it.
                if let Some(profile) = self.ephemeral_profile.take() {
                    tracing::warn!(dir = %profile.display(), "retaining startup profile because process exit is unconfirmed");
                }
            }
        }
        if let Some(profile) = self.ephemeral_profile.take() {
            if let Err(error) = std::fs::remove_dir_all(&profile) {
                tracing::debug!(%error, dir = %profile.display(), "failed to remove startup profile");
            }
        }
    }
}

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
        let mut startup = StartupGuard::new(ephemeral.then(|| user_data_dir.clone()));
        let stderr_log = if chrome.captures_stderr() {
            Some(if ephemeral {
                // Startup failure/cancellation removes the profile; retain its diagnostics.
                std::env::temp_dir().join(format!("rustwright-chrome-{}.log", unique_suffix()))
            } else {
                user_data_dir.join("rustwright-chrome.log")
            })
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

        startup.own_child(command.spawn().map_err(|source| BrowserError::Launch {
            executable: executable.clone(),
            source,
        })?);

        let (endpoint, ws_url) = match &active_port_dir {
            Some(dir) => {
                wait_for_active_port(
                    startup.child_mut(),
                    dir,
                    &executable,
                    chrome.startup_timeout_value(),
                    &stderr_log,
                )
                .await?
            }
            None => {
                wait_for_http_endpoint(
                    startup.child_mut(),
                    fixed_port,
                    chrome.startup_timeout_value(),
                    &stderr_log,
                )
                .await?
            }
        };

        Ok(Self {
            child: startup.ready(),
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
    ///
    /// Ephemeral launches keep this log outside the temporary profile so startup
    /// errors retain readable diagnostics after profile cleanup. The log is
    /// retained on startup failure/cancellation. A successfully returned ephemeral owner removes
    /// the log on Drop; persistent-profile logs remain inside that profile.
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
            if let Some(log) = &self.stderr_log {
                if let Err(error) = std::fs::remove_file(log) {
                    tracing::debug!(%error, path = %log.display(), "failed to remove ephemeral browser log");
                }
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

#[cfg(all(test, unix))]
pub(crate) mod startup_tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    pub(crate) struct Fixture {
        pub(crate) root: PathBuf,
        pub(crate) executable: PathBuf,
        pid_file: PathBuf,
        args_file: PathBuf,
    }

    impl Fixture {
        pub(crate) fn new(exit_early: bool) -> Self {
            let root = std::env::temp_dir().join(format!("rw-startup-owner-{}", unique_suffix()));
            std::fs::create_dir(&root).unwrap();
            let executable = root.join("owned-process");
            let end = if exit_early {
                "exit 17"
            } else {
                "exec sleep 60"
            };
            std::fs::write(&executable, format!("#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$RW_STARTUP_ARGS\"\nprintf 'startup-owner-fixture\\n' >&2\nprintf '%s\\n' \"$$\" > \"$RW_STARTUP_PID\"\n{end}\n")).unwrap();
            std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
            Self {
                executable,
                pid_file: root.join("pid"),
                args_file: root.join("args"),
                root,
            }
        }

        pub(crate) fn chrome(&self) -> Chrome {
            Chrome::at(&self.executable)
                .env("RW_STARTUP_PID", self.pid_file.to_string_lossy())
                .env("RW_STARTUP_ARGS", self.args_file.to_string_lossy())
        }

        pub(crate) fn firefox(&self) -> crate::Firefox {
            crate::Firefox::at(&self.executable)
                .env("RW_STARTUP_PID", self.pid_file.to_string_lossy())
                .env("RW_STARTUP_ARGS", self.args_file.to_string_lossy())
        }

        pub(crate) async fn started(&self) -> u32 {
            tokio::time::timeout(Duration::from_secs(3), async {
                loop {
                    if let Ok(value) = std::fs::read_to_string(&self.pid_file) {
                        if let Ok(pid) = value.trim().parse() {
                            return pid;
                        }
                    }
                    tokio::time::sleep(Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("controlled process must record its own PID")
        }

        pub(crate) fn chrome_profile(&self) -> PathBuf {
            std::fs::read_to_string(&self.args_file)
                .unwrap()
                .lines()
                .find_map(|arg| arg.strip_prefix("--user-data-dir=").map(PathBuf::from))
                .expect("launcher supplied a profile")
        }

        pub(crate) fn persistent_profile(&self) -> PathBuf {
            let profile = self.root.join("caller-profile");
            std::fs::create_dir(&profile).unwrap();
            std::fs::write(profile.join("sentinel"), "caller data").unwrap();
            profile
        }

        pub(crate) fn assert_cleanup(
            &self,
            pid: u32,
            case: &str,
            profile: &Path,
            owned: bool,
            log: Option<&Path>,
        ) {
            let alive = process_exists(pid);
            let profile_exists = profile.exists();
            let sentinel_preserved = profile.join("sentinel").is_file();
            let log_preserved = log.map(|p| {
                std::fs::read_to_string(p).is_ok_and(|s| s.contains("startup-owner-fixture"))
            });
            println!(
                "{}",
                serde_json::json!({"case":case,"pid":pid,"owned_process_still_present":alive,
                "profile":profile,"launcher_owned_profile":owned,"profile_exists":profile_exists,
                "caller_sentinel_preserved":sentinel_preserved,"error_log":log,"error_log_preserved":log_preserved})
            );
            // Independently terminate the controlled child even when the regression fails.
            if alive {
                kill_fixture(pid);
            }
            if owned {
                let _ = std::fs::remove_dir_all(profile);
            }
            if let Some(log) = log {
                let _ = std::fs::remove_file(log);
            }
            assert!(!alive, "{case}: startup child must be killed and reaped");
            assert_eq!(
                profile_exists, !owned,
                "{case}: profile ownership must be respected"
            );
            if !owned {
                assert!(sentinel_preserved, "{case}: caller data must remain");
            }
            if log.is_some() {
                assert_eq!(
                    log_preserved,
                    Some(true),
                    "{case}: failure log must remain readable"
                );
            }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            if let Ok(value) = std::fs::read_to_string(&self.pid_file) {
                if let Ok(pid) = value.trim().parse() {
                    if process_exists(pid) {
                        kill_fixture(pid);
                    }
                }
            }
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn process_exists(pid: u32) -> bool {
        Command::new("kill")
            .args(["-0", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .expect("Unix kill utility")
            .success()
    }
    fn kill_fixture(pid: u32) {
        let _ = Command::new("kill")
            .args(["-KILL", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
    pub(crate) fn error_log(error: &BrowserError) -> PathBuf {
        match error {
            BrowserError::StartupTimeout { log_path, .. }
            | BrowserError::EarlyExit { log_path, .. } => log_path.clone(),
            other => panic!("expected readiness failure, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn chrome_startup_timeout_reaps_child_and_removes_ephemeral_profile() {
        let fixture = Fixture::new(false);
        let error =
            LaunchedBrowser::launch(&fixture.chrome().startup_timeout(Duration::from_millis(250)))
                .await
                .unwrap_err();
        let pid = fixture.started().await;
        fixture.assert_cleanup(
            pid,
            "chrome_timeout",
            &fixture.chrome_profile(),
            true,
            Some(&error_log(&error)),
        );
    }

    #[tokio::test]
    async fn chrome_startup_cancellation_reaps_child_and_removes_ephemeral_profile() {
        let fixture = Fixture::new(false);
        let options = fixture.chrome();
        let launch = tokio::spawn(async move { LaunchedBrowser::launch(&options).await });
        let pid = fixture.started().await;
        launch.abort();
        assert!(launch.await.unwrap_err().is_cancelled());
        fixture.assert_cleanup(
            pid,
            "chrome_cancellation",
            &fixture.chrome_profile(),
            true,
            None,
        );
    }

    #[tokio::test]
    async fn chrome_startup_timeout_preserves_caller_profile() {
        let fixture = Fixture::new(false);
        let profile = fixture.persistent_profile();
        let error = LaunchedBrowser::launch(
            &fixture
                .chrome()
                .profile(&profile)
                .startup_timeout(Duration::from_millis(250)),
        )
        .await
        .unwrap_err();
        let pid = fixture.started().await;
        fixture.assert_cleanup(
            pid,
            "chrome_persistent_timeout",
            &profile,
            false,
            Some(&error_log(&error)),
        );
    }

    #[tokio::test]
    async fn chrome_early_exit_removes_ephemeral_profile_but_preserves_error_log() {
        let fixture = Fixture::new(true);
        let error = LaunchedBrowser::launch(&fixture.chrome())
            .await
            .unwrap_err();
        let pid = fixture.started().await;
        fixture.assert_cleanup(
            pid,
            "chrome_early_exit",
            &fixture.chrome_profile(),
            true,
            Some(&error_log(&error)),
        );
    }

    #[tokio::test]
    async fn chrome_ready_child_and_profile_transfer_to_returned_owner() {
        let fixture = Fixture::new(false);
        let options = fixture.chrome();
        let launch = tokio::spawn(async move { LaunchedBrowser::launch(&options).await });
        let pid = fixture.started().await;
        let profile = fixture.chrome_profile();
        std::fs::write(
            profile.join("DevToolsActivePort"),
            "54321\n/devtools/browser/ownership-fixture\n",
        )
        .unwrap();
        let mut browser = tokio::time::timeout(Duration::from_secs(3), launch)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        let log = browser.stderr_log().unwrap().to_path_buf();
        assert_eq!(browser.pid(), pid);
        assert!(browser.is_running());
        assert!(profile.exists());
        assert!(std::fs::read_to_string(&log)
            .unwrap()
            .contains("startup-owner-fixture"));
        drop(browser);
        fixture.assert_cleanup(pid, "chrome_ready_owner_drop", &profile, true, None);
        assert!(!log.exists(), "successful ephemeral owner removes its log");
    }
}
