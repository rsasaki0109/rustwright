//! Context ownership and page discovery regressions in an installed browser.

use std::time::Duration;

use rustwright::prelude::*;
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

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
    let url = format!("http://{}/page.html", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let mut sockets = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (mut socket, _) = accepted.unwrap();
                    sockets.spawn(async move {
                        let mut request = Vec::new();
                        loop {
                            let mut bytes = [0; 2048];
                            let Ok(count) = socket.read(&mut bytes).await else { return; };
                            if count == 0 { return; }
                            request.extend_from_slice(&bytes[..count]);
                            if request.windows(4).any(|s| s == b"\r\n\r\n") { break; }
                        }
                        let body = "<!DOCTYPE html><title>context</title><body>ready</body>";
                        let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        let _ = socket.write_all(response.as_bytes()).await;
                    });
                },
                _ = sockets.join_next(), if !sockets.is_empty() => {},
            }
        }
    });
    Fixture { url, task }
}

async fn settled() {
    // Give the waiter time to finish its initial target snapshot; subsequent
    // opens exercise event handling rather than just discovery of existing tabs.
    tokio::time::sleep(Duration::from_millis(100)).await;
}

async fn launch() -> Browser {
    Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("installed Chrome must start")
}

#[tokio::test]
async fn default_discovery_does_not_adopt_isolated_pages() {
    let browser = launch().await;
    let default_page = browser.new_page().await.unwrap();
    let isolated = browser.new_context().await.unwrap();
    let isolated_page = isolated.new_page().await.unwrap();
    let discovered = browser.pages().await.unwrap();
    assert!(discovered
        .iter()
        .any(|p| p.target_id() == default_page.target_id()));
    assert!(
        discovered
            .iter()
            .all(|p| p.target_id() != isolated_page.target_id()),
        "default discovery must preserve isolated page ownership: {discovered:?}"
    );
    browser.default_context().close().await.unwrap();
    assert_eq!(
        isolated_page.evaluate("6 * 7").await.unwrap(),
        serde_json::json!(42)
    );
    isolated.close().await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn closed_pages_are_removed_from_the_context_list() {
    let browser = launch().await;
    let context = browser.new_context().await.unwrap();
    let page = context.new_page().await.unwrap();
    page.close().await.unwrap();
    assert!(
        context.pages().is_empty(),
        "closed pages must not remain tracked"
    );
    assert!(context.refresh_pages().await.unwrap().is_empty());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn context_close_wakes_a_pending_page_wait() {
    let browser = launch().await;
    let context = browser.new_context().await.unwrap();
    let waiting = context.clone();
    let task = tokio::spawn(async move { waiting.wait_for_page(Duration::from_secs(5)).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    context.close().await.unwrap();
    let result = tokio::time::timeout(Duration::from_millis(500), task)
        .await
        .expect("context closure must wake the waiter")
        .unwrap();
    assert!(matches!(result, Err(Error::ContextClosed)), "{result:?}");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn browser_disconnect_wakes_a_pending_page_wait() {
    let browser = launch().await;
    let context = browser.new_context().await.unwrap();
    let task = tokio::spawn(async move { context.wait_for_page(Duration::from_secs(5)).await });
    tokio::time::sleep(Duration::from_millis(100)).await;
    browser.close().await.unwrap();
    let result = tokio::time::timeout(Duration::from_millis(500), task)
        .await
        .expect("browser disconnection must wake the waiter")
        .unwrap();
    assert!(matches!(result, Err(Error::BrowserClosed)), "{result:?}");
}

#[tokio::test]
async fn connected_browser_excludes_another_clients_isolated_context() {
    let browser = launch().await;
    let isolated = browser.new_context().await.unwrap();
    let isolated_page = isolated.new_page().await.unwrap();
    let connected = Browser::connect(&browser.version().web_socket_debugger_url)
        .await
        .unwrap();
    assert!(connected
        .pages()
        .await
        .unwrap()
        .iter()
        .all(|p| p.target_id() != isolated_page.target_id()));
    connected.default_context().close().await.unwrap();
    assert_eq!(isolated_page.evaluate("42").await.unwrap(), json!(42));
    connected.close().await.unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn default_popup_wait_ignores_isolated_target_created_events() {
    let fixture = fixture().await;
    let browser = launch().await;
    let opener = browser.new_page().await.unwrap();
    opener.goto(&fixture.url).await.unwrap();
    browser.pages().await.unwrap(); // Adopt the browser's initial default tab.
    let context = browser.default_context();
    let waiting = context.clone();
    let task = tokio::spawn(async move { waiting.wait_for_page(Duration::from_secs(3)).await });
    settled().await;
    let isolated = browser.new_context().await.unwrap();
    let foreign = isolated.new_page().await.unwrap();
    settled().await;
    assert!(
        !task.is_finished(),
        "an isolated page is not a default popup"
    );
    let url = format!("{}#default-popup", fixture.url);
    opener
        .evaluate(&format!("window.open({url:?}); true"))
        .await
        .unwrap();
    let popup = task.await.unwrap().unwrap();
    assert_ne!(popup.target_id(), foreign.target_id());
    popup
        .wait_for_url_with_timeout("#default-popup", Duration::from_secs(3))
        .await
        .unwrap();
    assert_eq!(popup.title().await.unwrap(), "context");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn isolated_popup_wait_ignores_default_and_other_isolated_contexts() {
    let fixture = fixture().await;
    let browser = launch().await;
    let context = browser.new_context().await.unwrap();
    let opener = context.new_page().await.unwrap();
    opener.goto(&fixture.url).await.unwrap();
    let waiting = context.clone();
    let task = tokio::spawn(async move { waiting.wait_for_page(Duration::from_secs(3)).await });
    settled().await;
    browser.new_page().await.unwrap();
    let other = browser.new_context().await.unwrap();
    other.new_page().await.unwrap();
    settled().await;
    assert!(
        !task.is_finished(),
        "other contexts must not satisfy the wait"
    );
    let url = format!("{}#isolated-popup", fixture.url);
    opener
        .evaluate(&format!("window.open({url:?}); true"))
        .await
        .unwrap();
    let popup = task.await.unwrap().unwrap();
    popup
        .wait_for_url_with_timeout("#isolated-popup", Duration::from_secs(3))
        .await
        .unwrap();
    assert_eq!(context.pages().len(), 2);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn concurrent_waiters_adopt_distinct_popups() {
    let fixture = fixture().await;
    let browser = launch().await;
    let context = browser.new_context().await.unwrap();
    let opener = context.new_page().await.unwrap();
    opener.goto(&fixture.url).await.unwrap();
    let mut tasks = JoinSet::new();
    for _ in 0..2 {
        let waiting = context.clone();
        tasks.spawn(async move { waiting.wait_for_page(Duration::from_secs(3)).await });
    }
    settled().await;
    // Chrome grants one popup per user gesture. Each evaluate call supplies a
    // fresh gesture; assert creation so popup blocking cannot mimic a lost event.
    for fragment in ["one", "two"] {
        assert_eq!(
            opener
                .evaluate(&format!(
                    "window.open({:?}) !== null",
                    format!("{}#{fragment}", fixture.url)
                ))
                .await
                .unwrap(),
            json!(true)
        );
    }
    let first = tasks.join_next().await.unwrap().unwrap().unwrap();
    let second = tasks.join_next().await.unwrap().unwrap().unwrap();
    assert_ne!(first.target_id(), second.target_id());
    assert_eq!(context.pages().len(), 3);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cancelled_popup_wait_does_not_stop_future_discovery() {
    let fixture = fixture().await;
    let browser = launch().await;
    let context = browser.new_context().await.unwrap();
    let opener = context.new_page().await.unwrap();
    opener.goto(&fixture.url).await.unwrap();
    let waiting = context.clone();
    let task = tokio::spawn(async move { waiting.wait_for_page(Duration::from_secs(5)).await });
    settled().await;
    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    opener
        .evaluate(&format!(
            "window.open({:?}); true",
            format!("{}#after-cancel", fixture.url)
        ))
        .await
        .unwrap();
    let popup = context.wait_for_page(Duration::from_secs(3)).await.unwrap();
    popup
        .wait_for_url_with_timeout("#after-cancel", Duration::from_secs(3))
        .await
        .unwrap();
    let timed_out = context.wait_for_page(Duration::from_millis(120)).await;
    assert!(
        matches!(timed_out, Err(Error::Timeout { .. })),
        "{timed_out:?}"
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn concurrent_refresh_shares_one_page_session_and_route_state() {
    let fixture = fixture().await;
    let browser = launch().await;
    let context = browser.new_context().await.unwrap();
    let connection =
        rustwright::cdp::CdpConnection::connect(&browser.version().web_socket_debugger_url)
            .await
            .unwrap();
    let target = connection
        .send_raw(
            None,
            "Target.createTarget",
            json!({
                "url": fixture.url, "browserContextId": context.browser_context_id(),
            }),
        )
        .await
        .unwrap();
    let target_id = target["targetId"].as_str().unwrap();
    let mut tasks = JoinSet::new();
    for _ in 0..6 {
        let context = context.clone();
        tasks.spawn(async move { context.refresh_pages().await.unwrap() });
    }
    let mut pages = Vec::new();
    while let Some(result) = tasks.join_next().await {
        let discovered = result.unwrap();
        assert_eq!(discovered.len(), 1);
        pages.push(discovered.into_iter().next().unwrap());
    }
    assert!(pages.iter().all(|p| p.target_id() == target_id));
    pages[0]
        .wait_for_load_state_with_timeout(LoadState::Load, Duration::from_secs(3))
        .await
        .unwrap();
    pages[0]
        .mock("**/shared-route", 201, "text/plain", "shared mock")
        .await
        .unwrap();
    assert_eq!(
        pages[1]
            .evaluate("fetch('/shared-route').then(r => r.status)")
            .await
            .unwrap(),
        json!(201)
    );
    pages[2].clear_routes().await.unwrap();
    assert_eq!(
        pages[0]
            .evaluate("fetch('/shared-route').then(r => r.status)")
            .await
            .unwrap(),
        json!(200)
    );
    connection.close();
    browser.close().await.unwrap();
}
