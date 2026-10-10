//! Actual process loss, pending operation cleanup and manual restart.
//! Every iteration starts a fresh browser; startup failures never skip.

use std::{
    path::PathBuf,
    result::Result as TestResult,
    sync::atomic::{AtomicU64, Ordering},
    time::{Duration, Instant},
};

use rustwright::{
    bidi::{BidiBrowser, BidiError},
    browser::{LaunchedBrowser, LaunchedFirefox},
    cdp::CdpConnection,
    prelude::*,
    AnyError,
};
use serde_json::{json, Value};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::broadcast,
    task::{JoinHandle, JoinSet},
};

const CYCLES: usize = 25;
const DEADLINE: Duration = Duration::from_secs(2);
static PROFILE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    url: String,
    profile: PathBuf,
    task: JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

async fn fixture() -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/crash.html", listener.local_addr().unwrap());
    let profile = std::env::temp_dir().join(format!(
        "rustwright-disconnect-{}-{}",
        std::process::id(),
        PROFILE.fetch_add(1, Ordering::Relaxed)
    ));
    let task = tokio::spawn(async move {
        let mut requests = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (mut socket, _) = accepted.unwrap();
                    requests.spawn(async move {
                        let mut request = Vec::new();
                        loop {
                            let mut bytes = [0; 2048];
                            let Ok(n) = socket.read(&mut bytes).await else { return; };
                            if n == 0 { return; }
                            request.extend_from_slice(&bytes[..n]);
                            if request.len() > 16 * 1024 { return; }
                            if request.windows(4).any(|b| b == b"\r\n\r\n") { break; }
                        }
                        let body = "<!DOCTYPE html><title>crash</title><input id=q>";
                        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        let _ = socket.write_all(response.as_bytes()).await;
                    });
                },
                _ = requests.join_next(), if !requests.is_empty() => {},
            }
        }
    });
    Fixture { url, profile, task }
}

// Observe that both promises entered the browser before killing it. The helper
// wrapper preserves the original locator wait rather than replacing its result.
async fn begin_pending(
    page: &AnyPage,
) -> (
    JoinHandle<TestResult<Value, AnyError>>,
    JoinHandle<TestResult<(), AnyError>>,
) {
    assert_eq!(page.title().await.unwrap(), "crash");
    page.locator("#q").fill("restart 日本語").await.unwrap();
    assert_eq!(
        page.evaluate("document.querySelector('#q').value")
            .await
            .unwrap(),
        json!("restart 日本語")
    );
    page.evaluate("(() => { const original = window.__rustwright.waitFor; window.__rustwright.waitFor = (...args) => { window.waitStarted = true; return original(...args); }; return true; })()").await.unwrap();
    let evaluating = page.clone();
    let evaluation = tokio::spawn(async move {
        evaluating
            .evaluate("window.evalStarted = true; new Promise(() => {})")
            .await
    });
    let locating = page.clone();
    let wait = tokio::spawn(async move {
        locating
            .locator("#absent")
            .wait_for_with_timeout(WaitState::Visible, Duration::from_secs(20))
            .await
    });
    tokio::time::timeout(DEADLINE, async {
        loop {
            if page
                .evaluate("window.waitStarted === true && window.evalStarted === true")
                .await
                .unwrap()
                == json!(true)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("both operations must enter the browser before process loss");
    (evaluation, wait)
}

fn assert_closed(error: AnyError) {
    let closed = match &error {
        AnyError::Chrome(error) => error.is_closed(),
        AnyError::Firefox(error) => matches!(error, BidiError::Closed),
    };
    assert!(
        closed,
        "process loss must report closure, not a wait timeout: {error:?}"
    );
}

async fn pending_closed(
    evaluation: JoinHandle<TestResult<Value, AnyError>>,
    wait: JoinHandle<TestResult<(), AnyError>>,
) {
    tokio::time::timeout(DEADLINE, async {
        let (evaluation, wait) = tokio::join!(evaluation, wait);
        assert_closed(evaluation.unwrap().unwrap_err());
        assert_closed(wait.unwrap().unwrap_err());
    })
    .await
    .expect("pending operations must wake within two seconds of process loss");
}

async fn channel_closed<T: Clone>(
    mut events: broadcast::Receiver<T>,
    marker: impl Fn(&T) -> bool,
) -> usize {
    tokio::time::timeout(DEADLINE, async {
        let mut markers = 0;
        loop {
            match events.recv().await {
                Ok(event) => markers += usize::from(marker(&event)),
                Err(broadcast::error::RecvError::Closed) => return markers,
                Err(error) => panic!("unexpected event loss: {error}"),
            }
        }
    })
    .await
    .expect("event senders must be released after all connection owners drop")
}

#[tokio::test]
async fn chrome_process_loss_and_manual_restart() {
    for cycle in 0..CYCLES {
        let fixture = fixture().await;
        let mut process = LaunchedBrowser::launch(&Chrome::installed().headless(true))
            .await
            .expect("Chrome must start; no skip");
        let browser = Browser::connect(process.ws_url()).await.unwrap();
        // Chrome's high-level connection is private. Use an independent observer
        // for channel lifetime; the pending operations use the Browser connection.
        let observer = CdpConnection::connect(process.ws_url()).await.unwrap();
        let events = observer.subscribe();
        let page: AnyPage = browser.new_page().await.unwrap().into();
        page.goto(&fixture.url).await.unwrap();
        let (evaluation, wait) = begin_pending(&page).await;
        assert!(browser.pending_command_count() >= 2);
        let started = Instant::now();
        process.kill(); // No graceful Browser.close or protocol close handshake.
        assert!(!process.is_running());
        pending_closed(evaluation, wait).await;
        assert_eq!(browser.pending_command_count(), 0);
        assert!(!browser.is_connected());
        assert_closed(
            tokio::time::timeout(DEADLINE, page.title())
                .await
                .unwrap()
                .unwrap_err(),
        );
        let wake_ms = started.elapsed().as_secs_f64() * 1000.0;
        let version = browser.version().browser.clone();
        let pid = process.pid();
        drop(page);
        drop(browser);
        drop(observer);
        assert_eq!(
            channel_closed(events, |event| event.is_disconnected()).await,
            1
        );
        eprintln!(
            "{}",
            json!({"engine":"chrome", "cycle":cycle + 1, "version":version, "pid":pid, "pending_after_loss":0, "wake_ms":wake_ms, "cleanup_ms":started.elapsed().as_secs_f64()*1000.0, "root_exited":true, "event_channel_closed":true})
        );
    }
}

#[tokio::test]
async fn firefox_process_loss_and_manual_restart() {
    for cycle in 0..CYCLES {
        let fixture = fixture().await;
        let mut process = LaunchedFirefox::launch(
            &Firefox::installed()
                .headless(true)
                .profile(&fixture.profile),
        )
        .await
        .expect("Firefox must start; no skip");
        let browser = BidiBrowser::connect(process.ws_url()).await.unwrap();
        let connection = browser.session().connection().clone();
        let events = connection.subscribe();
        let page: AnyPage = browser.new_page().await.unwrap().into();
        page.goto(&fixture.url).await.unwrap();
        let (evaluation, wait) = begin_pending(&page).await;
        assert!(connection.pending_command_count() >= 2);
        let started = Instant::now();
        process.kill(); // No graceful session.end or protocol close handshake.
        assert!(!process.is_running());
        pending_closed(evaluation, wait).await;
        assert_eq!(connection.pending_command_count(), 0);
        assert!(connection.is_closed());
        assert_closed(
            tokio::time::timeout(DEADLINE, page.title())
                .await
                .unwrap()
                .unwrap_err(),
        );
        let wake_ms = started.elapsed().as_secs_f64() * 1000.0;
        let version = browser.browser_version().map(str::to_owned);
        let pid = process.pid();
        drop(page);
        drop(browser);
        drop(connection);
        assert_eq!(channel_closed(events, |_| false).await, 0);
        eprintln!(
            "{}",
            json!({"engine":"firefox", "cycle":cycle + 1, "version":version, "pid":pid, "pending_after_loss":0, "wake_ms":wake_ms, "cleanup_ms":started.elapsed().as_secs_f64()*1000.0, "root_exited":true, "event_channel_closed":true})
        );
    }
}
