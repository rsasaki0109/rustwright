//! Identical HTTP scenarios for installed Chrome/CDP and Firefox/BiDi.
//! Browser discovery or startup failure fails these tests; neither backend skips.

use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

use rustwright::{bidi::BidiBrowser, prelude::*};
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

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
        "rustwright-http-actionability-{}-{}",
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
                            if n == 0 || request.len() + n > 16 * 1024 { return; }
                            request.extend_from_slice(&bytes[..n]);
                            if request.windows(4).any(|b| b == b"\r\n\r\n") { break; }
                        }
                        let text = String::from_utf8_lossy(&request);
                        let path = text.lines().next().unwrap().split_whitespace().nth(1).unwrap().split('?').next().unwrap();
                        let body = if path == "/frame.html" { FRAME } else { HTML };
                        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
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
        task,
    }
}

enum Engine {
    Chrome(Browser),
    Firefox(BidiBrowser),
}

impl Engine {
    async fn close(self) {
        match self {
            Self::Chrome(b) => b.close().await.unwrap(),
            Self::Firefox(b) => b.close().await.unwrap(),
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
        let page = browser.new_page().await.unwrap().into();
        (Engine::Firefox(browser), page)
    } else {
        let browser = Browser::launch(Chrome::installed().headless(true))
            .await
            .expect("Chrome must start; no skip");
        eprintln!("Chrome version: {}", browser.version().browser);
        let page = browser.new_page().await.unwrap().into();
        (Engine::Chrome(browser), page)
    };
    page.goto(&format!("{}/page.html", fixture.base))
        .await
        .unwrap();
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

async fn action(firefox: bool, setup: &str, expected: serde_json::Value) {
    let (_fixture, browser, page) = open(firefox).await;
    page.evaluate(setup).await.unwrap();
    let clicked = tokio::time::timeout(Duration::from_secs(3), page.locator("#go").click()).await;
    let observed = page
        .evaluate("[window.clicked,window.trusted,window.ready,window.wrong || 0]")
        .await
        .unwrap();
    browser.close().await;
    assert!(matches!(clicked, Ok(Ok(()))), "click failed: {clicked:?}");
    assert_eq!(observed, expected);
}

const TRACK: &str = "window.clicked=0;window.ready=false;window.wrong=0;document.querySelector('#go').onclick=e=>{window.clicked++;window.trusted=e.isTrusted;if(!window.ready)window.wrong++};";

async fn disabled_fieldset(firefox: bool) {
    action(firefox, &format!("{TRACK}const f=document.createElement('fieldset');f.disabled=true;document.body.append(f);f.append(document.querySelector('#go'));setTimeout(()=>{{f.disabled=false;window.ready=true}},600)"), json!([1,true,true,0])).await;
}
both!(
    chrome_disabled_fieldset,
    firefox_disabled_fieldset,
    disabled_fieldset
);

async fn disabled_aria_ancestor(firefox: bool) {
    action(firefox, &format!("{TRACK}document.body.setAttribute('aria-disabled','true');setTimeout(()=>{{document.body.removeAttribute('aria-disabled');window.ready=true}},600)"), json!([1,true,true,0])).await;
}
both!(
    chrome_disabled_aria_ancestor,
    firefox_disabled_aria_ancestor,
    disabled_aria_ancestor
);

async fn transient_overlay(firefox: bool) {
    action(firefox, &format!("{TRACK}const cover=document.createElement('div');cover.style='position:fixed;inset:0;z-index:999;background:white';cover.onclick=()=>window.wrong++;document.body.append(cover);setTimeout(()=>{{cover.remove();window.ready=true}},600)"), json!([1,true,true,0])).await;
}
both!(
    chrome_transient_overlay,
    firefox_transient_overlay,
    transient_overlay
);

async fn move_on_hover(firefox: bool) {
    action(firefox, &format!("{TRACK}const button=document.querySelector('#go');button.style='position:fixed;left:100px;top:100px';document.addEventListener('mousemove',()=>{{if(!window.ready)button.style.left=button.style.left==='100px'?'400px':'100px'}});setTimeout(()=>window.ready=true,600)"), json!([1,true,true,0])).await;
}
both!(chrome_move_on_hover, firefox_move_on_hover, move_on_hover);

async fn click_timeout(firefox: bool, blocked_script: bool) {
    let (_fixture, browser, page) = open(firefox).await;
    let script = if blocked_script {
        "window.__rustwright.prepareClick=()=>new Promise(()=>{});true"
    } else {
        "document.querySelector('#go').disabled=true;true"
    };
    page.evaluate(script).await.unwrap();
    let started = tokio::time::Instant::now();
    let timeout = Duration::from_millis(if blocked_script { 120 } else { 500 });
    let error = match page.locator("#go") {
        rustwright::AnyLocator::Chrome(locator) => locator
            .click_with_timeout(timeout)
            .await
            .unwrap_err()
            .to_string(),
        rustwright::AnyLocator::Firefox(locator) => locator
            .click_with_timeout(timeout)
            .await
            .unwrap_err()
            .to_string(),
    };
    let elapsed = started.elapsed();
    let clicked = page.evaluate("window.clicked").await.unwrap();
    // A timed-out browser-side promise does not keep the client command ledger.
    let pending = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            let pending = match &browser {
                Engine::Chrome(browser) => browser.pending_command_count(),
                Engine::Firefox(browser) => browser.session().connection().pending_command_count(),
            };
            if pending == 0 {
                return pending;
            }
            // CDP's remote-object guard releases its object asynchronously.
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    browser.close().await;
    assert!(
        elapsed >= timeout && elapsed < Duration::from_secs(1),
        "deadline took {elapsed:?}"
    );
    assert!(error.contains("timed out"), "unexpected error: {error}");
    if !blocked_script {
        assert!(
            error.contains("disabled"),
            "missing actionability reason: {error}"
        );
    }
    assert_eq!(clicked, json!(0));
    assert_eq!(pending, Ok(0));
}

async fn disabled_timeout(firefox: bool) {
    click_timeout(firefox, false).await;
}
both!(
    chrome_disabled_timeout,
    firefox_disabled_timeout,
    disabled_timeout
);
async fn blocked_prepare_timeout(firefox: bool) {
    click_timeout(firefox, true).await;
}
both!(
    chrome_blocked_prepare_timeout,
    firefox_blocked_prepare_timeout,
    blocked_prepare_timeout
);
