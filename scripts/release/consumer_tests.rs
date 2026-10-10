//! Consumer checks copied into a project resolved from the actual .crate files.
use rustwright::prelude::*;
use serde_json::{json, Value};
use std::{path::PathBuf, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

const LIMIT: Duration = Duration::from_secs(10);
const HTML: &str = "<!DOCTYPE html><title>Packaged Rustwright</title><h1>Welcome</h1><input name=q placeholder=Search><button id=go onclick=\"window.clicks=(window.clicks||0)+1;window.trusted=event.isTrusted;document.querySelector('#out').textContent='Results'\">Submit</button><div id=out>waiting</div><ul><li class=item>one</li><li class=item>two</li><li class=item>three</li></ul>";
struct Fixture {
    url: String,
    task: JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
async fn fixture() -> Fixture {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}/page", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut clients = JoinSet::new();
        loop {
            tokio::select! {
                accepted=listener.accept()=> {
                    let (mut socket,_)=accepted.unwrap();
                    clients.spawn(async move {
                        let mut request=Vec::new();
                        loop {
                            let mut buffer=[0;2048];
                            let Ok(n)=socket.read(&mut buffer).await else {return};
                            if n==0 {return;}
                            request.extend_from_slice(&buffer[..n]);
                            if request.len()>16384 {return;}
                            if request.windows(4).any(|b|b==b"\r\n\r\n") {break;}
                        }
                        let api=String::from_utf8_lossy(&request).lines().next().is_some_and(|line|line.starts_with("GET /api "));
                        let body=if api {"live"} else {HTML};
                        let response=format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                        let _=socket.write_all(response.as_bytes()).await;
                    });
                }
                _=clients.join_next(),if !clients.is_empty()=>{}
            }
        }
    });
    Fixture { url, task }
}

fn selected() -> String {
    std::env::var("RUSTWRIGHT_BROWSER").unwrap_or_else(|_| "chrome".into())
}
fn required_browser() {
    let key = if selected() == "firefox" {
        "RUSTWRIGHT_FIREFOX"
    } else {
        "RUSTWRIGHT_CHROME"
    };
    let path = std::env::var_os(key)
        .expect("consumer validation requires an explicit installed browser; no discovery skip");
    assert!(
        PathBuf::from(path).is_file(),
        "configured browser must exist"
    );
    assert_eq!(std::env::var("RUSTWRIGHT_HEADLESS").as_deref(), Ok("1"));
    assert_eq!(std::env::var("RUSTWRIGHT_RETRIES").as_deref(), Ok("0"));
    assert!(
        std::env::var_os("RUSTWRIGHT_SHARD").is_none(),
        "consumer checks must execute every test"
    );
}
fn screenshot_path(label: &str) -> PathBuf {
    let directory = std::env::var_os("RUSTWRIGHT_RELEASE_OUTPUT")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    std::fs::create_dir_all(&directory).unwrap();
    directory.join(format!("{}-{label}-{}.png", selected(), std::process::id()))
}
fn valid_png(path: &PathBuf) {
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes.len() > 24);
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap());
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap());
    assert!(
        width >= 200 && height >= 200,
        "native page screenshot dimensions: {width}x{height}"
    );
}

// This function is extracted verbatim from the README by the release harness.
#[path = "../src/readme_generic.rs"]
mod readme_generic;
use readme_generic::fill_and_read;

async fn ui(page: &AnyPage, url: &str, label: &str) {
    let text = "配布アーカイブ Rust 🦀";
    assert_eq!(
        tokio::time::timeout(LIMIT, fill_and_read(page, url, text))
            .await
            .expect("packaged injected helper must load and fill")
            .unwrap(),
        text
    );
    assert_eq!(page.title().await.unwrap(), "Packaged Rustwright");
    page.get_by_role(Role::Button, Some("Submit"))
        .click()
        .await
        .unwrap();
    rustwright_test::expect(page.get_by_text("Results"))
        .to_be_visible()
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("({clicks:window.clicks,trusted:window.trusted})")
            .await
            .unwrap(),
        json!({"clicks":1,"trusted":true})
    );
    let screenshot = screenshot_path(label);
    page.screenshot(&screenshot).await.unwrap();
    valid_png(&screenshot);
    println!(
        "{}",
        json!({"case":label,"backend":page.backend_name(),"unicode_input":text,"trusted_clicks":1,"screenshot":screenshot,"png_verified":true})
    );
}

enum Engine {
    Chrome(Browser),
    Firefox(BidiBrowser),
}
impl Engine {
    async fn launch() -> Self {
        let profile = std::env::var_os("RUSTWRIGHT_PROFILE")
            .map(PathBuf::from)
            .expect("disposable consumer profile required");
        if selected() == "firefox" {
            Self::Firefox(
                BidiBrowser::launch(Firefox::installed().headless(true).profile(profile))
                    .await
                    .expect("Firefox must launch; no skip"),
            )
        } else {
            Self::Chrome(
                Browser::launch(Chrome::installed().headless(true).profile(profile))
                    .await
                    .expect("Chrome must launch; no skip"),
            )
        }
    }
    async fn page(&self) -> AnyPage {
        match self {
            Self::Chrome(b) => b.new_page().await.unwrap().into(),
            Self::Firefox(b) => b.new_page().await.unwrap().into(),
        }
    }
    fn pending(&self) -> usize {
        match self {
            Self::Chrome(b) => b.pending_command_count(),
            Self::Firefox(b) => b.session().connection().pending_command_count(),
        }
    }
    async fn close(self) {
        match self {
            Self::Chrome(b) => b.close().await.unwrap(),
            Self::Firefox(b) => b.close().await.unwrap(),
        }
    }
}
async fn fetched(page: &AnyPage) -> Value {
    tokio::time::timeout(LIMIT, page.evaluate("fetch('/api').then(r=>r.text())"))
        .await
        .expect("packaged interception must not strand request")
        .unwrap()
}

#[tokio::test]
async fn archived_facade_helpers_routing_and_screenshot() {
    required_browser();
    let fixture = fixture().await;
    let engine = Engine::launch().await;
    let page = engine.page().await;
    ui(&page, &fixture.url, "facade").await;
    match &page {
        AnyPage::Chrome(p) => p
            .mock("*/api", 200, "text/plain", "packaged")
            .await
            .unwrap(),
        AnyPage::Firefox(p) => p
            .mock("*/api", 200, "text/plain", "packaged")
            .await
            .unwrap(),
    }
    assert_eq!(fetched(&page).await, json!("packaged"));
    match &page {
        AnyPage::Chrome(p) => p.clear_routes().await.unwrap(),
        AnyPage::Firefox(p) => p.clear_routes().await.unwrap(),
    }
    assert_eq!(fetched(&page).await, json!("live"));
    assert_eq!(engine.pending(), 0);
    page.close().await.unwrap();
    drop(page);
    engine.close().await;
}

#[rustwright_test::rustwright_test]
async fn archived_proc_macro_runner(
    context: rustwright_test::TestContext,
) -> rustwright_test::Result<()> {
    required_browser();
    assert_eq!(context.browser_name(), selected());
    let fixture = fixture().await;
    ui(&context.page, &fixture.url, "runner").await;
    rustwright_test::expect(context.page.locator("li.item"))
        .to_have_count(3)
        .await?;
    println!(
        "{}",
        json!({"case":"runner-complete","backend":context.browser_name(),"macro_callback_executed":true})
    );
    Ok(())
}
