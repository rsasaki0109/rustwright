use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::{Path, PathBuf},
    sync::{atomic::{AtomicBool, Ordering}, Arc},
    time::{Duration, Instant},
};
use rustwright::{Browser, Chrome};
use rustwright::firefox::{BidiBrowser, Firefox};
use serde_json::{json, Value};

enum OwnedBrowser { Chrome(Browser), Firefox(BidiBrowser) }
impl OwnedBrowser {
    async fn close(self) {
        match self {
            Self::Chrome(browser) => { let _ = browser.close().await; }
            Self::Firefox(browser) => { let _ = browser.close().await; }
        }
    }
}

fn arguments(path: &Path) -> Vec<String> {
    fs::read(path).unwrap_or_default().split(|b| *b == 0)
        .filter(|b| !b.is_empty()).map(|b| String::from_utf8_lossy(b).into_owned()).collect()
}
fn status(pid: i32) -> Option<String> { fs::read_to_string(format!("/proc/{pid}/status")).ok() }

async fn cycle(root: &Path, backend: &str, index: usize, launcher: &Path, native: &Path) -> Value {
    let directory = root.join(format!("{backend}-{index:02}"));
    fs::create_dir(&directory).unwrap();
    let temporary = directory.join("owned-tmp");
    fs::create_dir(&temporary).unwrap();
    // Restrict temporary profiles/logs to this fixture's owned directory.
    // This does not delay startup or change any browser command-line flags.
    std::env::set_var("TMPDIR", &temporary);
    let profile = directory.join("caller-profile");
    let sentinel = b"caller-owned startup cancellation sentinel\n";
    if backend == "firefox" {
        fs::create_dir(&profile).unwrap();
        fs::write(profile.join("sentinel"), sentinel).unwrap();
    }
    let wrapper = directory.join("record-and-exec");
    let source = format!("#!/bin/sh\nprintf '%s\\n' \"$$\" > '{}'\nprintf '%s\\0' \"$@\" > '{}'\nexec '{}' \"$@\"\n",
        directory.join("pid").display(), directory.join("args.bin").display(), launcher.display());
    fs::write(&wrapper, source).unwrap();
    fs::set_permissions(&wrapper, fs::Permissions::from_mode(0o700)).unwrap();
    let handoff = Arc::new(AtomicBool::new(false));
    let signal = handoff.clone();
    let backend_owned = backend.to_owned();
    let launch_profile = profile.clone();
    let task = tokio::spawn(async move {
        let result = if backend_owned == "chrome" {
            Browser::launch(Chrome::at(wrapper).headless(true).startup_timeout(Duration::from_secs(10)))
                .await.map(OwnedBrowser::Chrome).map_err(|error| error.to_string())
        } else {
            BidiBrowser::launch(Firefox::at(wrapper).headless(true).profile(launch_profile).startup_timeout(Duration::from_secs(10)))
                .await.map(OwnedBrowser::Firefox).map_err(|error| error.to_string())
        };
        signal.store(true, Ordering::Release);
        result
    });
    let started = Instant::now();
    let mut pid = None;
    let mut observed = None;
    let mut before = None;
    let mut reason = None;
    loop {
        if pid.is_none() { pid = fs::read_to_string(directory.join("pid")).ok().and_then(|s| s.trim().parse::<i32>().ok()); }
        if let Some(child) = pid {
            if let Ok(actual) = fs::read_link(format!("/proc/{child}/exe")) {
                if actual == native {
                    before = status(child);
                    observed = Some(actual);
                    // Current-thread runtime: there is no await between checking
                    // public handoff and aborting, so launch cannot run in between.
                    if handoff.load(Ordering::Acquire) || task.is_finished() { reason = Some("handoff_preceded_native_observation"); }
                    else { task.abort(); }
                    break;
                }
            }
        }
        if task.is_finished() { reason = Some("launch_finished_before_native_observation"); break; }
        if started.elapsed() > Duration::from_secs(10) { reason = Some("native_observation_deadline"); task.abort(); break; }
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    let wait = tokio::time::timeout(Duration::from_secs(6), task).await;
    let cancelled = matches!(&wait, Ok(Err(error)) if error.is_cancelled());
    let mut launch_error = None;
    match wait {
        Ok(Ok(Ok(browser))) => { browser.close().await; }
        Ok(Ok(Err(error))) => { launch_error = Some(error); }
        Ok(Err(error)) if !error.is_cancelled() => { launch_error = Some(error.to_string()); }
        Err(_) => { launch_error = Some("cancelled task did not finish within six seconds".into()); }
        _ => {}
    }
    let args = arguments(&directory.join("args.bin"));
    let chrome_profile = args.iter().find_map(|arg| arg.strip_prefix("--user-data-dir=").map(PathBuf::from));
    let direct_parent = before.as_deref().and_then(|s| s.lines().find_map(|l| l.strip_prefix("PPid:").and_then(|p| p.trim().parse::<u32>().ok())));
    let proc_absent = pid.is_some_and(|child| !Path::new(&format!("/proc/{child}")).exists());
    let mut waitpid_return = None;
    let mut waitpid_errno = None;
    let mut fixture_cleanup = Vec::new();
    if let Some(child) = pid {
        let mut child_status = 0;
        // ECHILD means the Rustwright owner already waited, rather than leaving
        // a zombie for this observer to reap. Any observer reaping is a failure.
        let returned = unsafe { libc::waitpid(child, &mut child_status, libc::WNOHANG) };
        waitpid_return = Some(returned);
        waitpid_errno = if returned == -1 { std::io::Error::last_os_error().raw_os_error() } else { None };
        if returned == 0 && direct_parent == Some(std::process::id()) {
            unsafe { libc::kill(child, libc::SIGKILL); libc::waitpid(child, &mut child_status, 0); }
            fixture_cleanup.push("observer killed and reaped its own remaining direct child after failed assertion");
        } else if returned > 0 {
            fixture_cleanup.push("observer reaped child after failed assertion");
        }
    }
    let profile_ok = if backend == "chrome" {
        chrome_profile.as_ref().is_some_and(|p| p.starts_with(&temporary) && !p.exists())
    } else {
        profile.is_dir() && fs::read(profile.join("sentinel")).ok().as_deref() == Some(sentinel)
    };
    let success = reason.is_none() && cancelled && launch_error.is_none() && observed.is_some()
        && direct_parent == Some(std::process::id()) && proc_absent
        && waitpid_return == Some(-1) && waitpid_errno == Some(libc::ECHILD) && profile_ok;
    let record = json!({"backend":backend,"cycle":index,"success":success,"pid":pid,
        "expected_native":native,"observed_native_before_cancel":observed,
        "pre_cancel_proc_status":before,"observer_pid":std::process::id(),"direct_parent":direct_parent,
        "launch_handoff_reached":handoff.load(Ordering::Acquire),"launch_task_cancelled":cancelled,
        "proc_absent_after_cancel":proc_absent,"waitpid_return":waitpid_return,"waitpid_errno":waitpid_errno,
        "profile_ownership_assertion":profile_ok,"chrome_ephemeral_profile":chrome_profile,
        "firefox_caller_profile":if backend=="firefox" {Some(&profile)} else {None},
        "firefox_sentinel":if backend=="firefox" {Some(String::from_utf8_lossy(sentinel))} else {None},
        "launch_arguments":args,"reason":reason,"launch_error":launch_error,"fixture_cleanup":fixture_cleanup,
        "scope":"actual direct browser child and profile ownership only; no descendant/process-tree or latency guarantee"});
    fs::write(directory.join("result.json"), serde_json::to_vec_pretty(&record).unwrap()).unwrap();
    // Caller profile cleanup is fixture-owned and happens only after the
    // preservation assertion/result has been recorded.
    if profile.exists() { fs::remove_dir_all(&profile).unwrap(); }
    record
}

#[tokio::main(flavor="current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().collect();
    assert_eq!(args.len(), 6, "ROOT BACKEND COUNT CONTAINER_LAUNCHER ACTUAL_NATIVE");
    let root = Path::new(&args[1]);
    fs::create_dir(root).unwrap();
    let backend = &args[2];
    assert!(backend == "chrome" || backend == "firefox");
    let count: usize = args[3].parse().unwrap();
    let mut records = Vec::new();
    for index in 0..count {
        let record = cycle(root, backend, index, Path::new(&args[4]), Path::new(&args[5])).await;
        println!("{}", record);
        let passed = record["success"] == true;
        records.push(record);
        if !passed { break; }
    }
    let success = records.len() == count && records.iter().all(|r| r["success"] == true);
    fs::write(root.join("summary.json"), serde_json::to_vec_pretty(&json!({"success":success,"requested":count,"completed":records.len(),"records":records})).unwrap()).unwrap();
    if !success { std::process::exit(1); }
}
