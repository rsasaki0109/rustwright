//! Linux process ownership plus a controllable CDP peer; no installed browser.
use super::*;
use futures_util::{SinkExt, StreamExt};
use serde_json::Value;
use std::{
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{net::TcpListener, sync::mpsc, task::JoinHandle};
use tokio_tungstenite::{accept_async, tungstenite::Message};

struct Remote {
    root: PathBuf,
    commands: mpsc::UnboundedReceiver<Value>,
    replies: mpsc::UnboundedSender<Value>,
    task: JoinHandle<()>,
    external: Option<LaunchedBrowser>,
}
impl Drop for Remote {
    fn drop(&mut self) {
        self.task.abort();
        self.external.take();
        let _ = std::fs::remove_dir_all(&self.root);
    }
}
async fn pair(owned: bool) -> (Browser, Remote, u32, PathBuf) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let root = std::env::temp_dir().join(format!(
        "rustwright-shutdown-test-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    std::fs::create_dir(&root).unwrap();
    let executable = root.join("fake-chrome");
    std::fs::write(&executable, "#!/bin/sh\nfor argument in \"$@\"; do\n case \"$argument\" in --user-data-dir=*) profile=${argument#--user-data-dir=};; esac\ndone\nprintf '%s\\n/session\\n' \"$RUSTWRIGHT_TEST_PORT\" > \"$profile/DevToolsActivePort\"\nexec /bin/sleep 600\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
    let (tx, commands) = mpsc::unbounded_channel();
    let (replies, mut rx) = mpsc::unbounded_channel::<Value>();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut socket = accept_async(stream).await.unwrap();
        loop {
            tokio::select! {
                incoming = socket.next() => {
                    let Some(Ok(Message::Text(text))) = incoming else {break};
                    let command: Value = serde_json::from_str(&text).unwrap();
                    let result = match command["method"].as_str().unwrap() {
                        "Browser.getVersion" => json!({"protocolVersion":"1.3","product":"Mock/1","revision":"0","userAgent":"mock","jsVersion":"1"}),
                        "Target.setDiscoverTargets" => json!({}),
                        "Browser.close" => {tx.send(command).unwrap(); continue;},
                        method => panic!("unexpected command {method}"),
                    };
                    socket.send(Message::Text(json!({"id":command["id"],"result":result}).to_string().into())).await.unwrap();
                }
                reply = rx.recv() => {
                    let Some(command) = reply else {break};
                    socket.send(Message::Text(json!({"id":command["id"],"result":{}}).to_string().into())).await.unwrap();
                }
            }
        }
    });
    let chrome = Chrome::at(&executable).env("RUSTWRIGHT_TEST_PORT", port.to_string());
    let (browser, external, pid, profile) = if owned {
        let browser = Browser::launch(chrome).await.unwrap();
        let diagnostics = browser.diagnostics();
        let pid = diagnostics.pid.unwrap();
        let profile = diagnostics.user_data_dir.unwrap();
        (browser, None, pid, profile)
    } else {
        let launched = LaunchedBrowser::launch(&chrome).await.unwrap();
        let pid = launched.pid();
        let profile = launched.user_data_dir().to_owned();
        let browser = Browser::connect(launched.ws_url()).await.unwrap();
        (browser, Some(launched), pid, profile)
    };
    (
        browser,
        Remote {
            root,
            commands,
            replies,
            task,
            external,
        },
        pid,
        profile,
    )
}
fn running(pid: u32) -> bool {
    PathBuf::from(format!("/proc/{pid}")).exists()
}
fn closing(browser: &Browser) -> JoinHandle<Result<()>> {
    let browser = browser.clone();
    tokio::spawn(async move { browser.close_original_for_regression().await })
}
async fn command(remote: &mut Remote) -> Value {
    tokio::time::timeout(Duration::from_secs(1), remote.commands.recv())
        .await
        .unwrap()
        .unwrap()
}
async fn finished(task: JoinHandle<Result<()>>) {
    tokio::time::timeout(Duration::from_secs(7), task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn cancelled_owned_shutdown_keeps_worker_alive_until_acknowledgement() {
    let (browser, mut remote, pid, profile) = pair(true).await;
    let first = closing(&browser);
    let close = command(&mut remote).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    assert!(
        running(pid),
        "cancelled waiter must not prematurely kill owned process"
    );
    assert_eq!(browser.pending_command_count(), 1);
    let mut second = closing(&browser);
    assert!(tokio::time::timeout(Duration::from_millis(40), &mut second)
        .await
        .is_err());
    remote.replies.send(close).unwrap();
    finished(second).await;
    assert!(!running(pid));
    assert!(!profile.exists());
    assert!(!browser.is_connected());
    assert_eq!(browser.pending_command_count(), 0);
    browser.close().await.unwrap();
    assert!(remote.commands.try_recv().is_err());
}

#[tokio::test]
async fn concurrent_owned_shutdown_does_not_interrupt_graceful_command() {
    let (browser, mut remote, pid, profile) = pair(true).await;
    let first = closing(&browser);
    let close = command(&mut remote).await;
    let mut second = closing(&browser);
    assert!(tokio::time::timeout(Duration::from_millis(40), &mut second)
        .await
        .is_err());
    assert!(running(pid));
    assert_eq!(browser.pending_command_count(), 1);
    remote.replies.send(close).unwrap();
    finished(first).await;
    finished(second).await;
    assert!(!running(pid));
    assert!(!profile.exists());
    assert!(!browser.is_connected());
    assert_eq!(browser.pending_command_count(), 0);
    assert!(remote.commands.try_recv().is_err());
}

#[tokio::test]
async fn cancelled_owned_shutdown_has_bounded_forced_process_and_socket_cleanup() {
    let (browser, mut remote, pid, profile) = pair(true).await;
    let first = closing(&browser);
    command(&mut remote).await;
    first.abort();
    assert!(first.await.unwrap_err().is_cancelled());
    assert!(running(pid));
    finished(closing(&browser)).await;
    assert!(!running(pid));
    assert!(!profile.exists());
    assert!(!browser.is_connected());
    assert_eq!(browser.pending_command_count(), 0);
}

#[tokio::test]
async fn externally_owned_process_survives_repeated_disconnect() {
    let (browser, mut remote, pid, profile) = pair(false).await;
    let first = closing(&browser);
    let second = closing(&browser);
    finished(first).await;
    finished(second).await;
    browser.close().await.unwrap();
    assert!(!browser.is_connected());
    assert!(running(pid));
    assert!(profile.exists());
    assert_eq!(browser.pending_command_count(), 0);
    assert!(
        remote.commands.try_recv().is_err(),
        "never send Browser.close for external process"
    );
    assert!(remote.external.as_mut().unwrap().is_running());
}

// Exact pre-change close implementation, used only while capturing baseline.
impl Browser {
    async fn close_original_for_regression(&self) -> Result<()> {
        let launched = self.process.lock().expect("process mutex poisoned").take();
        if let Some(mut launched) = launched {
            let _ = tokio::time::timeout(
                Duration::from_secs(5),
                self.connection.send_raw(None, "Browser.close", json!({})),
            )
            .await;
            launched.kill();
        }
        self.connection.close();
        Ok(())
    }
}
