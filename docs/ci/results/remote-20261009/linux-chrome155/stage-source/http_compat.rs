//! Identical HTTP scenarios for installed Chrome/CDP and Firefox/BiDi.
//! Browser discovery or startup failure fails these tests; neither backend skips.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use rustwright::{
    bidi::{BidiBrowser, BidiContext},
    prelude::*,
    AnyError,
};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

#[path = "support/chrome_navigation_probe.rs"]
mod chrome_navigation_probe;

static PROFILE: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    base: String,
    profile: std::path::PathBuf,
    task: JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
        let _ = std::fs::remove_dir_all(&self.profile);
    }
}

const HTML: &str = "<!DOCTYPE html><title>compat</title><style>body{margin:0} iframe{position:absolute;left:240px;top:160px;width:360px;height:200px;border:0}</style><input id=q><button id=go>Go</button><div id=out>waiting</div><script>window.clicked=0; document.querySelector('#go').onclick=e=>{window.clicked++;window.trusted=e.isTrusted;document.querySelector('#out').textContent='clicked'}</script>";
const FRAME: &str = "<!DOCTYPE html><title>frame</title><style>body{margin:0}</style><input id=q><button id=go>Go</button><script>window.clicked=0; document.querySelector('#go').onclick=e=>{window.clicked++;window.trusted=e.isTrusted}</script>";

async fn fixture() -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let profile = std::env::temp_dir().join(format!(
        "rustwright-http-compat-{}-{}",
        std::process::id(),
        PROFILE.fetch_add(1, Ordering::Relaxed)
    ));
    let task = tokio::spawn(async move {
        let mut requests = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (mut socket, peer) = accepted.unwrap();
                    eprintln!("HTTP compat fixture accepted: {peer}");
                    requests.spawn(async move {
                        let mut request = Vec::new();
                        loop {
                            let mut bytes = [0; 2048];
                            let Ok(n) = socket.read(&mut bytes).await else { return; };
                            if n == 0 || request.len() + n > 16 * 1024 { return; }
                            request.extend_from_slice(&bytes[..n]);
                            if request.windows(4).any(|b| b == b"\r\n\r\n") { break; }
                        }
                        let text = String::from_utf8_lossy(&request);
                        eprintln!("HTTP compat fixture request: peer={peer}, bytes={}, line={:?}", request.len(), text.lines().next());
                        let path = text.lines().next().unwrap().split_whitespace().nth(1).unwrap().split('?').next().unwrap();
                        let body = if path == "/frame.html" { FRAME } else { HTML };
                        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        let written = socket.write_all(response.as_bytes()).await;
                        eprintln!("HTTP compat fixture response: peer={peer}, bytes={}, content_type=text/html, write={written:?}", response.len());
                    });
                },
                _ = requests.join_next(), if !requests.is_empty() => {},
            }
        }
    });
    Fixture {
        base,
        profile,
        task,
    }
}

enum Engine {
    Chrome(Browser),
    Firefox(BidiBrowser),
}

impl Engine {
    async fn new_context(&self) -> TestContext {
        match self {
            Self::Chrome(b) => TestContext::Chrome(b.new_context().await.unwrap()),
            Self::Firefox(b) => TestContext::Firefox(b.new_context().await.unwrap()),
        }
    }
    async fn close(self) {
        match self {
            Self::Chrome(b) => b.close().await.unwrap(),
            Self::Firefox(b) => b.close().await.unwrap(),
        }
    }
}

enum TestContext {
    Chrome(BrowserContext),
    Firefox(BidiContext),
}

impl TestContext {
    async fn new_page(&self) -> AnyPage {
        match self {
            Self::Chrome(c) => c.new_page().await.unwrap().into(),
            Self::Firefox(c) => c.new_page().await.unwrap().into(),
        }
    }
    async fn pages(&self) -> Vec<AnyPage> {
        match self {
            Self::Chrome(c) => c
                .refresh_pages()
                .await
                .unwrap()
                .into_iter()
                .map(Into::into)
                .collect(),
            Self::Firefox(c) => c
                .pages()
                .await
                .unwrap()
                .into_iter()
                .map(Into::into)
                .collect(),
        }
    }
    async fn close(&self) {
        match self {
            Self::Chrome(c) => c.close().await.unwrap(),
            Self::Firefox(c) => c.close().await.unwrap(),
        }
    }
}

async fn open(firefox: bool) -> (Fixture, Engine, AnyPage) {
    let fixture = fixture().await;
    let (browser, page): (Engine, AnyPage) = if firefox {
        let browser = BidiBrowser::launch(
            Firefox::installed()
                .headless(true)
                .profile(&fixture.profile),
        )
        .await
        .expect("Firefox must start; no skip");
        eprintln!("Firefox version: {:?}", browser.browser_version());
        eprintln!("HTTP compat Firefox stage: new_page start");
        let page = browser.new_page().await.unwrap().into();
        eprintln!("HTTP compat Firefox stage: new_page complete");
        (Engine::Firefox(browser), page)
    } else {
        let browser = Browser::launch(Chrome::installed().headless(true))
            .await
            .expect("Chrome must start; no skip");
        eprintln!("Chrome version: {}", browser.version().browser);
        eprintln!("HTTP compat Chrome stage: new_page start");
        let page = match tokio::time::timeout(Duration::from_secs(30), browser.new_page()).await {
            Ok(result) => result.unwrap().into(),
            Err(error) => {
                eprintln!("HTTP compat Chrome new_page timed out: {error:?}");
                chrome_navigation_probe::diagnose_creation(&browser).await;
                panic!("Initial Chrome page creation exceeded 30 seconds: {error:?}");
            }
        };
        eprintln!("HTTP compat Chrome stage: new_page complete");
        (Engine::Chrome(browser), page)
    };
    let url = format!("{}/page.html", fixture.base);
    eprintln!("HTTP compat stage: goto start backend_firefox={firefox}, url={url}");
    if let Err(error) = page.goto(&url).await {
        eprintln!("Original HTTP compat navigation failure: {error:?}");
        if let (Engine::Chrome(browser), AnyPage::Chrome(page)) = (&browser, &page) {
            chrome_navigation_probe::diagnose(browser, page, &url).await;
        }
        panic!("Initial HTTP compat navigation failed: {error:?}");
    }
    eprintln!("HTTP compat stage: goto complete backend_firefox={firefox}");
    (fixture, browser, page)
}

macro_rules! both {
    ($chrome:ident, $firefox:ident, $scenario:ident) => {
        #[tokio::test]
        async fn $chrome() {
            $scenario(false).await;
        }
        #[tokio::test]
        async fn $firefox() {
            $scenario(true).await;
        }
    };
}

async fn navigation_and_forms(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    page.get_by_role(Role::Button, Some("Go"))
        .click()
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("[clicked,trusted]").await.unwrap(),
        json!([1, true])
    );
    page.locator("#q").fill("Rust 日本語").await.unwrap();
    assert_eq!(
        page.evaluate("document.querySelector('#q').value")
            .await
            .unwrap(),
        json!("Rust 日本語")
    );
    let url = format!("{}/page.html#anchor", fixture.base);
    page.goto(&url).await.unwrap();
    assert_eq!(page.url().await.unwrap(), url);
    page.evaluate("history.pushState({},'', '#pushed')")
        .await
        .unwrap();
    assert!(page.url().await.unwrap().ends_with("#pushed"));
    page.evaluate("new Promise(resolve=>{addEventListener('popstate',()=>resolve(true),{once:true});history.back()})").await.unwrap();
    assert_eq!(page.url().await.unwrap(), url);
    assert_eq!(page.title().await.unwrap(), "compat");
    browser.close().await;
}
both!(
    chrome_navigation_and_forms,
    firefox_navigation_and_forms,
    navigation_and_forms
);

async fn javascript_exceptions(firefox: bool) {
    let (_fixture, browser, page) = open(firefox).await;
    let result = page
        .evaluate("(() => { throw new Error('compat boom'); })()")
        .await;
    assert!(
        result.is_err(),
        "JavaScript exceptions must not become Ok(null): {result:?}"
    );
    assert!(result.unwrap_err().to_string().contains("compat boom"));
    let rejected = page
        .evaluate("Promise.reject(new Error('compat rejection'))")
        .await;
    assert!(rejected
        .unwrap_err()
        .to_string()
        .contains("compat rejection"));
    assert_eq!(page.evaluate("null").await.unwrap(), json!(null));
    assert_eq!(page.evaluate("undefined").await.unwrap(), json!(null));
    assert_eq!(
        page.evaluate("({x: [1, true, null]})").await.unwrap(),
        json!({"x": [1,true,null]})
    );
    browser.close().await;
}
both!(
    chrome_javascript_exceptions,
    firefox_javascript_exceptions,
    javascript_exceptions
);

async fn wait_deadline(firefox: bool) {
    let (_fixture, browser, page) = open(firefox).await;
    page.evaluate("window.__rustwright.waitFor = () => new Promise(() => {})")
        .await
        .unwrap();
    let result = tokio::time::timeout(
        Duration::from_millis(600),
        page.locator("#missing")
            .wait_for_with_timeout(WaitState::Visible, Duration::from_millis(120)),
    )
    .await
    .expect("locator timeout includes script.evaluate response");
    assert!(
        matches!(
            result,
            Err(AnyError::Chrome(Error::Timeout { .. }))
                | Err(AnyError::Firefox(rustwright::bidi::BidiError::WaitTimeout(
                    _
                )))
        ),
        "expected a driver deadline: {result:?}"
    );
    assert_eq!(page.title().await.unwrap(), "compat");
    browser.close().await;
}
both!(chrome_wait_deadline, firefox_wait_deadline, wait_deadline);

async fn frame_click(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    let url = format!("{}/frame.html", fixture.base).replace("127.0.0.1", "localhost");
    page.evaluate(&format!("new Promise(resolve => {{const f=document.createElement('iframe');f.id='child';f.onload=()=>resolve(true);f.src={url:?};document.body.append(f)}})")).await.unwrap();
    let value = match &page {
        AnyPage::Chrome(page) => {
            let frame = page.frame_locator("#child").resolve().await.unwrap();
            frame.locator("#go").click().await.unwrap();
            frame.evaluate("[clicked,trusted]").await.unwrap()
        }
        AnyPage::Firefox(page) => {
            let frame = page.frame_locator("#child").await.unwrap();
            frame.locator("#go").click().await.unwrap();
            frame.evaluate("[clicked,trusted]").await.unwrap()
        }
    };
    assert_eq!(
        value,
        json!([1, true]),
        "input must reach the cross-origin child"
    );
    browser.close().await;
}
both!(chrome_frame_click, firefox_frame_click, frame_click);

async fn frame_reload(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    let url = format!("{}/frame.html", fixture.base);
    page.evaluate(&format!("new Promise(resolve => {{const f=document.createElement('iframe');f.id='child';f.onload=()=>resolve(true);f.src={url:?};document.body.append(f)}})")).await.unwrap();
    match &page {
        AnyPage::Chrome(page) => {
            let frame = page.frame_locator("#child").resolve().await.unwrap();
            frame.locator("#q").fill("before").await.unwrap();
            page.evaluate("new Promise(resolve=>{const f=document.querySelector('#child');f.onload=()=>resolve(true);f.contentWindow.location.reload()})").await.unwrap();
            frame.locator("#q").fill("after").await.unwrap();
            assert_eq!(
                frame
                    .evaluate("document.querySelector('#q').value")
                    .await
                    .unwrap(),
                json!("after")
            );
        }
        AnyPage::Firefox(page) => {
            let frame = page.frame_locator("#child").await.unwrap();
            frame.locator("#q").fill("before").await.unwrap();
            page.evaluate("new Promise(resolve=>{const f=document.querySelector('#child');f.onload=()=>resolve(true);f.contentWindow.location.reload()})").await.unwrap();
            frame.locator("#q").fill("after").await.unwrap();
            assert_eq!(
                frame
                    .evaluate("document.querySelector('#q').value")
                    .await
                    .unwrap(),
                json!("after")
            );
        }
    }
    browser.close().await;
}
both!(chrome_frame_reload, firefox_frame_reload, frame_reload);

async fn closing_page_wakes_wait(firefox: bool) {
    let (_fixture, browser, page) = open(firefox).await;
    let waiting = page.locator("#missing");
    let task = tokio::spawn(async move {
        waiting
            .wait_for_with_timeout(WaitState::Visible, Duration::from_secs(5))
            .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    page.close().await.unwrap();
    let result = tokio::time::timeout(Duration::from_millis(600), task)
        .await
        .expect("closing the page wakes locator waits")
        .unwrap();
    assert!(result.is_err(), "closed page cannot satisfy a locator wait");
    browser.close().await;
}
both!(
    chrome_closing_page_wakes_wait,
    firefox_closing_page_wakes_wait,
    closing_page_wakes_wait
);

async fn delayed_and_absent_elements(firefox: bool) {
    let (_fixture, browser, page) = open(firefox).await;
    page.evaluate("setTimeout(()=>{const el=document.createElement('span');el.id='late';el.textContent='ready';document.body.append(el)},120)").await.unwrap();
    page.locator("#late")
        .wait_for_with_timeout(WaitState::Visible, Duration::from_secs(2))
        .await
        .unwrap();
    assert_eq!(page.locator("#late").text().await.unwrap(), "ready");
    page.evaluate("document.querySelector('#late').remove()")
        .await
        .unwrap();
    page.locator("#late")
        .wait_for_with_timeout(WaitState::Detached, Duration::from_secs(1))
        .await
        .unwrap();
    let result = page
        .locator("#absent")
        .wait_for_with_timeout(WaitState::Visible, Duration::from_millis(120))
        .await;
    assert!(result.is_err());
    browser.close().await;
}
both!(
    chrome_delayed_and_absent_elements,
    firefox_delayed_and_absent_elements,
    delayed_and_absent_elements
);

async fn cancelled_wait_recovers(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    let diagnostic_browser = match &browser {
        Engine::Chrome(browser) => Some(browser.clone()),
        Engine::Firefox(_) => None,
    };
    let scenario = async {
        eprintln!("Cancellation stage: replace wait helper start backend_firefox={firefox}");
        page.evaluate("window.savedWaitFor=window.__rustwright.waitFor;window.__rustwright.waitFor=()=>new Promise(()=>{})").await.unwrap();
        eprintln!("Cancellation stage: replace wait helper complete backend_firefox={firefox}");
        let locator = page.locator("#missing");
        let task = tokio::spawn(async move {
            locator
                .wait_for_with_timeout(WaitState::Visible, Duration::from_secs(5))
                .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        eprintln!("Cancellation stage: wait abort start backend_firefox={firefox}");
        task.abort();
        assert!(task.await.unwrap_err().is_cancelled());
        eprintln!("Cancellation stage: wait abort complete backend_firefox={firefox}");
        eprintln!("Cancellation stage: restore helper start backend_firefox={firefox}");
        page.evaluate("window.__rustwright.waitFor=window.savedWaitFor")
            .await
            .unwrap();
        eprintln!("Cancellation stage: restore helper complete backend_firefox={firefox}");
        eprintln!("Cancellation stage: visible wait start backend_firefox={firefox}");
        page.locator("#go")
            .wait_for_with_timeout(WaitState::Visible, Duration::from_secs(1))
            .await
            .unwrap();
        eprintln!("Cancellation stage: visible wait complete backend_firefox={firefox}");
        eprintln!("Cancellation stage: title start backend_firefox={firefox}");
        assert_eq!(page.title().await.unwrap(), "compat");
        eprintln!("Cancellation stage: title complete backend_firefox={firefox}");
        eprintln!("Cancellation stage: close start backend_firefox={firefox}");
        browser.close().await;
        eprintln!("Cancellation stage: close complete backend_firefox={firefox}");
    };
    if let Err(error) = tokio::time::timeout(Duration::from_secs(30), scenario).await {
        eprintln!("Cancellation scenario after open timed out: {error:?}");
        if let (Some(browser), AnyPage::Chrome(page)) = (&diagnostic_browser, &page) {
            chrome_navigation_probe::diagnose(
                browser,
                page,
                &format!("{}/page.html", fixture.base),
            )
            .await;
        }
        panic!("Cancellation scenario after open exceeded 30 seconds: {error:?}");
    }
}

both!(
    chrome_cancelled_wait_recovers,
    firefox_cancelled_wait_recovers,
    cancelled_wait_recovers
);

async fn disconnect_wakes_wait(firefox: bool) {
    let (_fixture, browser, page) = open(firefox).await;
    let locator = page.locator("#missing");
    let task = tokio::spawn(async move {
        locator
            .wait_for_with_timeout(WaitState::Visible, Duration::from_secs(5))
            .await
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    browser.close().await;
    let result = tokio::time::timeout(Duration::from_millis(600), task)
        .await
        .expect("disconnect wakes the waiter")
        .unwrap();
    assert!(result.is_err());
}
both!(
    chrome_disconnect_wakes_wait,
    firefox_disconnect_wakes_wait,
    disconnect_wakes_wait
);

async fn isolated_storage(firefox: bool) {
    let (fixture, browser, _default) = open(firefox).await;
    let first = browser.new_context().await;
    let second = browser.new_context().await;
    let a = first.new_page().await;
    let b = second.new_page().await;
    let url = format!("{}/page.html", fixture.base);
    a.goto(&url).await.unwrap();
    b.goto(&url).await.unwrap();
    a.evaluate("localStorage.setItem('rw','first');document.cookie='rw=first;path=/'")
        .await
        .unwrap();
    assert_eq!(
        b.evaluate("[localStorage.getItem('rw'),document.cookie]")
            .await
            .unwrap(),
        json!([null, ""])
    );
    assert_eq!(
        a.evaluate("[localStorage.getItem('rw'),document.cookie]")
            .await
            .unwrap(),
        json!(["first", "rw=first"])
    );
    first.close().await;
    assert!(a.title().await.is_err());
    assert_eq!(b.title().await.unwrap(), "compat");
    second.close().await;
    browser.close().await;
}
both!(
    chrome_isolated_storage,
    firefox_isolated_storage,
    isolated_storage
);

async fn isolated_popup_ownership(firefox: bool) {
    let (fixture, browser, _default) = open(firefox).await;
    let first = browser.new_context().await;
    let second = browser.new_context().await;
    let opener = first.new_page().await;
    let other = second.new_page().await;
    opener
        .goto(&format!("{}/page.html", fixture.base))
        .await
        .unwrap();
    other
        .goto(&format!("{}/page.html", fixture.base))
        .await
        .unwrap();
    let url = format!("{}/page.html#owned-popup", fixture.base);
    assert_eq!(
        opener
            .evaluate(&format!("window.open({url:?}) !== null"))
            .await
            .unwrap(),
        json!(true)
    );
    let pages = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            let pages = first.pages().await;
            if pages.len() == 2 {
                break pages;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("popup is discovered in its owner's context");
    assert_eq!(second.pages().await.len(), 1);
    let mut popup = None;
    for p in pages {
        if p.url().await.unwrap().contains("#owned-popup") {
            popup = Some(p);
        }
    }
    let popup = popup.expect("owner contains the expected popup");
    assert_eq!(popup.title().await.unwrap(), "compat");
    first.close().await;
    assert!(popup.title().await.is_err());
    assert_eq!(other.title().await.unwrap(), "compat");
    browser.close().await;
}
both!(
    chrome_isolated_popup_ownership,
    firefox_isolated_popup_ownership,
    isolated_popup_ownership
);

async fn frame_hover_and_keyboard(firefox: bool) {
    let (fixture, browser, page) = open(firefox).await;
    let url = format!("{}/frame.html", fixture.base).replace("127.0.0.1", "localhost");
    page.evaluate(&format!("new Promise(resolve=>{{const f=document.createElement('iframe');f.id='child';f.onload=()=>resolve(true);f.src={url:?};document.body.append(f)}})")).await.unwrap();
    let expression = "window.hovered=false;document.querySelector('#go').onmouseover=e=>window.hovered=e.isTrusted;window.typed=false;document.querySelector('#q').oninput=e=>window.typed=e.isTrusted";
    let result = match &page {
        AnyPage::Chrome(p) => {
            let f = p.frame_locator("#child").resolve().await.unwrap();
            f.evaluate(expression).await.unwrap();
            f.locator("#go").hover().await.unwrap();
            f.locator("#q").press("x").await.unwrap();
            f.locator("#q").type_text("yz").await.unwrap();
            f.evaluate("[hovered,typed,document.querySelector('#q').value]")
                .await
                .unwrap()
        }
        AnyPage::Firefox(p) => {
            let f = p.frame_locator("#child").await.unwrap();
            f.evaluate(expression).await.unwrap();
            f.locator("#go").hover().await.unwrap();
            f.locator("#q").press("x").await.unwrap();
            f.locator("#q").type_text("yz").await.unwrap();
            f.evaluate("[hovered,typed,document.querySelector('#q').value]")
                .await
                .unwrap()
        }
    };
    assert_eq!(result, json!([true, true, "xyz"]));
    browser.close().await;
}
both!(
    chrome_frame_hover_and_keyboard,
    firefox_frame_hover_and_keyboard,
    frame_hover_and_keyboard
);
