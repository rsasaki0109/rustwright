//! Shared Chrome/Firefox history and reload behavior over local HTTP.
use rustwright::{
    bidi::{BidiBrowser, BidiError},
    prelude::*,
    AnyPage,
};
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
    task::{JoinHandle, JoinSet},
};

const LIMIT: Duration = Duration::from_secs(5);
static NEXT_PROFILE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    base: String,
    profile: PathBuf,
    hold_image: Arc<AtomicBool>,
    entered: Arc<Notify>,
    release: Arc<Notify>,
    task: JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.release.notify_waiters();
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

async fn fixture() -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let profile = std::env::temp_dir().join(format!(
        "rustwright-history-{}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos(),
        NEXT_PROFILE.fetch_add(1, Ordering::Relaxed),
    ));
    std::fs::create_dir_all(&profile).unwrap();
    let hold_image = Arc::new(AtomicBool::new(false));
    let entered = Arc::new(Notify::new());
    let release = Arc::new(Notify::new());
    let (held, arrived, allowed) = (hold_image.clone(), entered.clone(), release.clone());
    let task = tokio::spawn(async move {
        let mut requests = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (mut socket, _) = accepted.unwrap();
                    let (held, arrived, allowed) = (held.clone(), arrived.clone(), allowed.clone());
                    requests.spawn(async move {
                        let mut bytes = Vec::new();
                        loop {
                            let mut chunk = [0; 2048];
                            let Ok(count) = socket.read(&mut chunk).await else { return; };
                            if count == 0 { return; }
                            bytes.extend_from_slice(&chunk[..count]);
                            if bytes.len() > 16 * 1024 { return; }
                            if bytes.windows(4).any(|part| part == b"\r\n\r\n") { break; }
                        }
                        let request = String::from_utf8_lossy(&bytes);
                        let path = request.lines().next().unwrap().split_whitespace().nth(1).unwrap().split('?').next().unwrap();
                        let (status, extra, mime, body) = match path {
                            "/a" | "/pushed-without-fragment" => ("200 OK", "", "text/html", "<!DOCTYPE html><title>A</title><input id=field>"),
                            "/b" => ("200 OK", "", "text/html", "<!DOCTYPE html><title>B</title><input id=field>"),
                            "/redirect" => ("302 Found", "Location: /b?redirected\r\n", "text/html", ""),
                            "/loading" => ("200 OK", "", "text/html", "<!DOCTYPE html><title>loading</title><input id=field><img src=/held.svg>"),
                            "/held.svg" => {
                                if held.load(Ordering::SeqCst) {
                                    arrived.notify_one();
                                    allowed.notified().await;
                                }
                                ("200 OK", "", "image/svg+xml", "<svg xmlns='http://www.w3.org/2000/svg'/>")
                            },
                            _ => ("404 Not Found", "", "text/html", ""),
                        };
                        let response = format!("HTTP/1.1 {status}\r\n{extra}Content-Type: {mime}; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        let _ = socket.write_all(response.as_bytes()).await;
                    });
                },
                _ = requests.join_next(), if !requests.is_empty() => {},
            }
        }
    });
    Fixture {
        base,
        profile,
        hold_image,
        entered,
        release,
        task,
    }
}

enum BrowserHandle {
    Chrome(Browser),
    Firefox(BidiBrowser),
}
impl BrowserHandle {
    async fn close(self) {
        match self {
            Self::Chrome(browser) => browser.close().await.unwrap(),
            Self::Firefox(browser) => browser.close().await.unwrap(),
        }
    }
}
async fn open(firefox: bool) -> (Fixture, BrowserHandle, AnyPage) {
    let fixture = fixture().await;
    let (browser, page) = if firefox {
        let browser = BidiBrowser::launch(
            Firefox::installed()
                .headless(true)
                .profile(&fixture.profile),
        )
        .await
        .expect("Firefox must launch; no skip");
        let page = browser.new_page().await.unwrap();
        println!("backend=firefox version={:?}", browser.browser_version());
        (BrowserHandle::Firefox(browser), AnyPage::Firefox(page))
    } else {
        let browser = Browser::launch(Chrome::installed().headless(true))
            .await
            .expect("Chrome must launch; no skip");
        let page = browser.new_page().await.unwrap();
        println!("backend=chrome version={}", browser.version().browser);
        (BrowserHandle::Chrome(browser), AnyPage::Chrome(page))
    };
    (fixture, browser, page)
}
async fn back(page: &AnyPage) {
    tokio::time::timeout(LIMIT, page.go_back())
        .await
        .unwrap()
        .unwrap();
}
async fn forward(page: &AnyPage) {
    tokio::time::timeout(LIMIT, page.go_forward())
        .await
        .unwrap()
        .unwrap();
}
async fn reload(page: &AnyPage) {
    page.reload().await.unwrap();
}

async fn history(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    page.goto(&format!("{}/a#one", fixture.base)).await.unwrap();
    page.evaluate("window.documentIdentity = 'original'; history.pushState({}, '', '#two')")
        .await
        .unwrap();
    back(&page).await;
    assert!(page.url().await.unwrap().ends_with("/a#one"));
    forward(&page).await;
    assert!(page.url().await.unwrap().ends_with("/a#two"));
    page.evaluate("history.pushState({}, '', '/pushed-without-fragment')")
        .await
        .unwrap();
    back(&page).await;
    assert!(page.url().await.unwrap().ends_with("/a#two"));
    forward(&page).await;
    assert!(
        page.url()
            .await
            .unwrap()
            .ends_with("/pushed-without-fragment"),
        "url={}",
        page.url().await.unwrap()
    );
    assert_eq!(
        page.evaluate("window.documentIdentity").await.unwrap(),
        "original"
    );
    page.goto(&format!("{}/redirect", fixture.base))
        .await
        .unwrap();
    assert!(page.url().await.unwrap().ends_with("/b?redirected"));
    back(&page).await;
    assert!(
        page.url()
            .await
            .unwrap()
            .ends_with("/pushed-without-fragment"),
        "url={}",
        page.url().await.unwrap()
    );
    assert_eq!(page.title().await.unwrap(), "A");
    forward(&page).await;
    assert_eq!(page.title().await.unwrap(), "B");
    assert_eq!(
        page.evaluate("document.readyState").await.unwrap(),
        "complete"
    );
    page.locator("#field").fill("履歴 日本語 🦀").await.unwrap();
    assert_eq!(
        page.evaluate("document.querySelector('#field').value")
            .await
            .unwrap(),
        "履歴 日本語 🦀"
    );
    browser.close().await;
}

async fn waits_for_reload(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    page.goto(&format!("{}/loading", fixture.base))
        .await
        .unwrap();
    page.evaluate("window.documentIdentity = 'old'")
        .await
        .unwrap();
    fixture.hold_image.store(true, Ordering::SeqCst);
    let mut pending = tokio::spawn({
        let page = page.clone();
        async move { reload(&page).await }
    });
    tokio::time::timeout(LIMIT, fixture.entered.notified())
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut pending)
            .await
            .is_err(),
        "reload must await the current document's held image"
    );
    fixture.hold_image.store(false, Ordering::SeqCst);
    fixture.release.notify_one();
    tokio::time::timeout(LIMIT, pending).await.unwrap().unwrap();
    assert_eq!(
        page.evaluate("typeof window.documentIdentity")
            .await
            .unwrap(),
        "undefined"
    );
    assert_eq!(
        page.evaluate("document.readyState").await.unwrap(),
        "complete"
    );
    page.locator("#field").fill("再読み込み 🦀").await.unwrap();
    assert_eq!(
        page.evaluate("document.querySelector('#field').value")
            .await
            .unwrap(),
        "再読み込み 🦀"
    );
    browser.close().await;
}

async fn boundary_and_closed(firefox: bool) {
    let (_fixture, browser, page) = open(firefox).await;
    let missing = match &page {
        AnyPage::Chrome(page) => page.go_forward().await.map_err(|e| e.to_string()),
        AnyPage::Firefox(page) => page.go_forward().await.map_err(|e| e.to_string()),
    };
    assert!(missing.is_err(), "a nonexistent forward entry must fail");
    if firefox {
        assert!(
            missing
                .as_ref()
                .unwrap_err()
                .contains("no such history entry"),
            "{missing:?}"
        );
    }
    page.close().await.unwrap();
    match &page {
        AnyPage::Chrome(page) => {
            assert!(page.reload().await.unwrap_err().is_closed());
            assert!(page.go_back().await.unwrap_err().is_closed());
            assert!(page.go_forward().await.unwrap_err().is_closed());
        }
        AnyPage::Firefox(page) => {
            for result in [
                page.reload().await,
                page.go_back().await,
                page.go_forward().await,
            ] {
                assert!(
                    matches!(result, Err(BidiError::Protocol { error, .. }) if error == "no such frame"),
                    "closed page retains the protocol's missing-context error"
                );
            }
        }
    }
    browser.close().await;
}

#[tokio::test]
async fn chrome_history_commits_and_loads() {
    history(false).await;
}
#[tokio::test]
async fn firefox_history_commits_and_loads() {
    history(true).await;
}
#[tokio::test]
async fn chrome_reload_waits_for_resources() {
    waits_for_reload(false).await;
}
#[tokio::test]
async fn firefox_reload_waits_for_resources() {
    waits_for_reload(true).await;
}
#[tokio::test]
async fn chrome_history_boundary_and_closed_errors() {
    boundary_and_closed(false).await;
}
#[tokio::test]
async fn firefox_history_boundary_and_closed_errors() {
    boundary_and_closed(true).await;
}

#[tokio::test]
async fn firefox_reload_deadline_cancellation_and_recovery() {
    let (fixture, browser, page) = open(true).await;
    let AnyPage::Firefox(page) = page else {
        unreachable!()
    };
    page.goto(&format!("{}/loading", fixture.base))
        .await
        .unwrap();
    fixture.hold_image.store(true, Ordering::SeqCst);
    let result = page.reload_with_timeout(Duration::from_millis(100)).await;
    assert!(
        matches!(result, Err(BidiError::Timeout { method, timeout }) if method == "browsingContext.reload" && timeout == Duration::from_millis(100))
    );
    tokio::time::timeout(LIMIT, fixture.entered.notified())
        .await
        .unwrap();
    let BrowserHandle::Firefox(ref bidi) = browser else {
        unreachable!()
    };
    assert_eq!(bidi.session().connection().pending_command_count(), 0);
    fixture.hold_image.store(false, Ordering::SeqCst);
    fixture.release.notify_one();
    page.goto(&format!("{}/a", fixture.base)).await.unwrap();
    assert_eq!(page.title().await.unwrap(), "A");
    page.goto(&format!("{}/loading", fixture.base))
        .await
        .unwrap();
    fixture.hold_image.store(true, Ordering::SeqCst);
    let cancelled = tokio::spawn({
        let page = page.clone();
        async move { page.reload().await }
    });
    tokio::time::timeout(LIMIT, fixture.entered.notified())
        .await
        .unwrap();
    cancelled.abort();
    assert!(cancelled.await.unwrap_err().is_cancelled());
    assert_eq!(bidi.session().connection().pending_command_count(), 0);
    fixture.hold_image.store(false, Ordering::SeqCst);
    fixture.release.notify_one();
    page.goto(&format!("{}/b", fixture.base)).await.unwrap();
    assert_eq!(page.title().await.unwrap(), "B");
    browser.close().await;
}
