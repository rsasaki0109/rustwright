//! Exercise the public runner in subprocesses so environment overrides stay local.

#![cfg(unix)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use rustwright_test::run_test;

struct TempDirectory(PathBuf);

impl TempDirectory {
    fn new() -> Self {
        static NEXT_ID: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "rustwright-runner-{}-{}",
            std::process::id(),
            NEXT_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("create runner fixture");
        Self(path)
    }
}

impl Drop for TempDirectory {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).expect("remove runner fixture");
    }
}

#[test]
fn runner_probe() {
    if std::env::var_os("RUSTWRIGHT_RUNNER_PROBE").is_none() {
        return;
    }
    run_test("startup_probe", |_context| async {
        panic!("test body must not run after failed setup");
    });
}

fn probe_command(browser: &str, executable: &Path, retries: u32) -> Command {
    let mut command = Command::new(std::env::current_exe().expect("test executable"));
    command
        .args(["--exact", "runner_probe", "--nocapture"])
        .env("RUSTWRIGHT_RUNNER_PROBE", "1")
        .env("RUSTWRIGHT_BROWSER", browser)
        .env("RUSTWRIGHT_CHROME", executable)
        .env("RUSTWRIGHT_FIREFOX", executable)
        .env("RUSTWRIGHT_HEADLESS", "1")
        .env("RUSTWRIGHT_RETRIES", retries.to_string())
        .env_remove("RUSTWRIGHT_SHARD")
        .env_remove("RUSTWRIGHT_PROFILE");
    command
}

fn assert_setup_fails(executable: &Path, reason: &str) {
    for browser in ["chrome", "firefox"] {
        let output = probe_command(browser, executable, 0)
            .output()
            .expect("run startup probe");
        let logs = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            !output.status.success(),
            "{browser} startup failure was skipped: {logs}"
        );
        assert!(logs.contains(&format!("{browser} setup failed:")), "{logs}");
        assert!(logs.contains(reason), "{logs}");
        assert!(!logs.contains("test body must not run"), "{logs}");
    }
}

#[test]
fn browser_exiting_during_startup_fails_the_test() {
    assert_setup_fails(Path::new("/bin/false"), "browser exited during startup");
}

#[test]
fn missing_explicit_browser_fails_instead_of_falling_back() {
    let fixture = TempDirectory::new();
    assert_setup_fails(
        &fixture.0.join("missing-browser"),
        "browser executable not found at",
    );
}

#[test]
fn non_executable_browser_fails_the_test() {
    let fixture = TempDirectory::new();
    let executable = fixture.0.join("not-executable");
    std::fs::write(&executable, "not a browser").expect("write non-executable browser");
    assert_setup_fails(&executable, "failed to launch browser");
}

#[test]
fn startup_failure_uses_the_configured_retries() {
    use std::os::unix::fs::PermissionsExt;

    let fixture = TempDirectory::new();
    let executable = fixture.0.join("failing-browser");
    std::fs::write(
        &executable,
        "#!/bin/sh\nprintf 'attempt\\n' >> \"$RUSTWRIGHT_RUNNER_ATTEMPT_FILE\"\nexit 1\n",
    )
    .expect("write failing browser");
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700))
        .expect("make browser executable");
    for browser in ["chrome", "firefox"] {
        let attempts = fixture.0.join(format!("{browser}-attempts"));
        let output = probe_command(browser, &executable, 1)
            .env("RUSTWRIGHT_RUNNER_ATTEMPT_FILE", &attempts)
            .output()
            .expect("run retry probe");
        let logs = String::from_utf8_lossy(&output.stderr);
        assert!(
            !output.status.success(),
            "startup should fail after retrying"
        );
        assert!(logs.contains("failed (attempt 1/2), retrying"), "{logs}");
        assert!(logs.contains("failed after 2 attempt(s)"), "{logs}");
        assert!(logs.contains(&format!("{browser} setup failed:")), "{logs}");
        assert_eq!(
            std::fs::read_to_string(attempts)
                .expect("attempt log")
                .lines()
                .count(),
            2
        );
    }
}
