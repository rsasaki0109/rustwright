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
        "rustwright-http-clipped-control-{}-{}",
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
    let headless = match std::env::var("RUSTWRIGHT_HEADLESS").as_deref() {
        Ok("0" | "false") => false,
        Ok("1" | "true") | Err(std::env::VarError::NotPresent) => true,
        value => panic!("RUSTWRIGHT_HEADLESS must be 0, false, 1 or true: {value:?}"),
    };
    #[cfg(target_os = "linux")]
    if !headless {
        assert!(
            ["DISPLAY", "WAYLAND_DISPLAY"]
                .iter()
                .any(|name| { std::env::var(name).is_ok_and(|display| !display.is_empty()) }),
            "Headed Linux browsers require DISPLAY or WAYLAND_DISPLAY"
        );
    }
    if firefox && !headless {
        assert!(
            std::env::var_os("MOZ_HEADLESS").is_none_or(|value| value.is_empty()),
            "Unset MOZ_HEADLESS to run Firefox with visible windows"
        );
    }
    eprintln!("Requested browser mode: headless={headless}");
    let fixture = fixture().await;
    let (browser, page): (Engine, AnyPage) = if firefox {
        let browser = BidiBrowser::launch(
            Firefox::installed()
                .headless(headless)
                .profile(&fixture.profile),
        )
        .await
        .expect("Firefox must start; no skip");
        eprintln!("Firefox version: {:?}", browser.browser_version());
        let page = browser.new_page().await.unwrap().into();
        (Engine::Firefox(browser), page)
    } else {
        let browser = Browser::launch(Chrome::installed().headless(headless))
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

async fn clipped_control(firefox: bool, width: u32, nested: bool, extra: &str) {
    let (_fixture, browser, page) = open(firefox).await;
    page.evaluate(&format!(r#"(() => {{
        window.clicked=0; window.wrong=0;
        const clip=document.createElement('div');
        clip.style='position:fixed;left:100px;top:100px;width:{width}px;height:60px;overflow:hidden';
        document.body.append(clip);
        const button=document.querySelector('#go');
        button.style='display:block;width:400px;height:50px';
        button.onclick=e=>{{window.clicked++;window.trusted=e.isTrusted}};
        clip.onclick=e=>{{if(e.target!==button)window.wrong++}};
        clip.append(button);
        if ({nested}) {{
            const outer=document.createElement('div');
            outer.style='position:fixed;left:100px;top:100px;width:20px;height:60px;overflow:hidden';
            document.body.append(outer);
            clip.style.position='relative'; clip.style.left='0'; clip.style.top='0';
            outer.append(clip);
        }}
        {extra}
        return true;
    }})()"#)).await.unwrap();
    let click = match page.locator("#go") {
        rustwright::AnyLocator::Chrome(locator) => locator
            .click_with_timeout(Duration::from_secs(3))
            .await
            .map_err(|e| e.to_string()),
        rustwright::AnyLocator::Firefox(locator) => locator
            .click_with_timeout(Duration::from_secs(3))
            .await
            .map_err(|e| e.to_string()),
    };
    let observed = page
        .evaluate("[window.clicked,window.trusted||false,window.wrong]")
        .await
        .unwrap();
    let geometry=page.evaluate("(()=>{const el=document.querySelector('#go'),p=el.parentElement;return {rect:el.getBoundingClientRect().toJSON(),clip:p.getBoundingClientRect().toJSON(),scroll:p.scrollLeft,hit:document.elementFromPoint(101,125)?.id}})()").await.unwrap();
    eprintln!(
        "width={width} nested={nested} click={click:?} observed={observed} geometry={geometry}"
    );
    browser.close().await;
    assert!(click.is_ok(), "clipped click failed: {click:?}; {geometry}");
    assert_eq!(observed, json!([1, true, 0]));
}
async fn wide(firefox: bool) {
    clipped_control(firefox, 60, false, "").await;
}
both!(
    chrome_overflow_hidden_control,
    firefox_overflow_hidden_control,
    wide
);
async fn narrow(firefox: bool) {
    clipped_control(firefox, 3, false, "").await;
}
both!(
    chrome_narrow_overflow_control,
    firefox_narrow_overflow_control,
    narrow
);
async fn nested(firefox: bool) {
    clipped_control(firefox, 60, true, "").await;
}
both!(
    chrome_nested_overflow_control,
    firefox_nested_overflow_control,
    nested
);

// Fixed descendants escape ordinary overflow ancestors; the conservative bounds
// must preserve their native hit points instead of inventing a paint clip.
async fn escaped_fixed(firefox: bool) {
    clipped_control(
        firefox,
        60,
        false,
        "button.style.position='fixed';button.style.left='300px';button.style.top='200px';",
    )
    .await;
}
both!(
    chrome_fixed_control_escapes_clip,
    firefox_fixed_control_escapes_clip,
    escaped_fixed
);

async fn document_scroll(firefox: bool) {
    clipped_control(firefox, 60, false, "clip.style.position='absolute';clip.style.left='900px';clip.style.top='900px';document.body.style.minWidth='1800px';document.body.style.minHeight='1800px';").await;
}
both!(
    chrome_scrolled_overflow_control,
    firefox_scrolled_overflow_control,
    document_scroll
);
