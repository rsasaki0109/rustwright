//! Network-idle regressions using controllable local HTTP response bodies.

use std::{sync::Arc, time::Duration};

use rustwright::prelude::*;
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Semaphore,
    task::{JoinHandle, JoinSet},
    time::Instant,
};

struct Fixture {
    base: String,
    bodies: Arc<Semaphore>,
    task: JoinHandle<()>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn open() -> (Fixture, Browser, Page) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let bodies = Arc::new(Semaphore::new(0));
    let held = bodies.clone();
    let task = tokio::spawn(async move {
        let mut requests = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (mut socket, _) = accepted.unwrap();
                    let bodies = held.clone();
                    requests.spawn(async move {
                        let mut request = Vec::new();
                        loop {
                            let mut buffer = [0; 2048];
                            let Ok(count) = socket.read(&mut buffer).await else { return; };
                            if count == 0 { return; }
                            request.extend_from_slice(&buffer[..count]);
                            if request.windows(4).any(|part| part == b"\r\n\r\n") { break; }
                        }
                        let text = String::from_utf8_lossy(&request);
                        let path = text.lines().next().unwrap().split_whitespace().nth(1).unwrap().split('?').next().unwrap();
                        if path == "/redirect" {
                            let _ = socket.write_all(b"HTTP/1.1 302 Found\r\nLocation: /stream?redirected\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;
                            return;
                        }
                        let (status, body) = match path {
                            "/page.html" | "/frame.html" => ("200 OK", "<!DOCTYPE html><title>idle fixture</title><body>ready</body>"),
                            "/fetch-frame.html" => ("200 OK", "<!DOCTYPE html><script>window.body = fetch('/stream?child').then(r => r.text());</script><body>child</body>"),
                            "/stream" => ("200 OK", "finished"),
                            _ => ("404 Not Found", ""),
                        };
                        let header = format!("HTTP/1.1 {status}\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                        if socket.write_all(header.as_bytes()).await.is_err() { return; }
                        if path == "/stream" { bodies.acquire_owned().await.unwrap().forget(); }
                        let _ = socket.write_all(body.as_bytes()).await;
                    });
                },
                _ = requests.join_next(), if !requests.is_empty() => {},
            }
        }
    });
    let fixture = Fixture { base, bodies, task };
    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .unwrap();
    let page = browser.new_page().await.unwrap();
    page.goto(&format!("{}/page.html", fixture.base))
        .await
        .unwrap();
    idle(&page).await;
    (fixture, browser, page)
}

async fn idle(page: &Page) {
    page.wait_for_load_state_with_timeout(LoadState::NetworkIdle, Duration::from_secs(3))
        .await
        .expect("network idle watchdog");
}

async fn wait_for_headers(page: &Page, marker: &str) {
    wait_for_header_count(page, marker, 1).await;
}

async fn wait_for_header_count(page: &Page, marker: &str, count: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if page
                .network_requests()
                .iter()
                .filter(|r| r.url.contains(marker) && r.status == Some(200))
                .count()
                >= count
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("request headers must be observed");
}

async fn assert_busy(page: &Page) {
    let result = page
        .wait_for_load_state_with_timeout(LoadState::NetworkIdle, Duration::from_millis(150))
        .await;
    assert!(
        matches!(result, Err(Error::Timeout { .. })),
        "unfinished HTTP response prevents idle: {result:?}"
    );
}

async fn assert_fresh_quiet_window(page: &Page) {
    let started = Instant::now();
    idle(page).await;
    assert!(
        started.elapsed() >= Duration::from_millis(450),
        "idle needs 500ms without requests, not an old lifecycle event: {:?}",
        started.elapsed()
    );
}

async fn open_frame(browser: &Browser, page: &Page, url: &str, id: &str, remote: bool) -> Frame {
    page.evaluate(&format!("new Promise(resolve => {{ const f = document.createElement('iframe'); f.id = {id:?}; f.onload = () => resolve(true); f.src = {url:?}; document.body.append(f); }})")).await.unwrap();
    let frame = page
        .frame_locator(format!("#{id}"))
        .resolve()
        .await
        .unwrap();
    if remote {
        assert_remote(browser, &frame).await;
    }
    frame
}

async fn assert_remote(browser: &Browser, frame: &Frame) {
    let connection =
        rustwright::cdp::CdpConnection::connect(&browser.version().web_socket_debugger_url)
            .await
            .unwrap();
    let targets = connection
        .send_raw(None, "Target.getTargets", json!({}))
        .await
        .unwrap();
    assert!(
        targets["targetInfos"]
            .as_array()
            .unwrap()
            .iter()
            .any(|target| target["type"] == "iframe" && target["targetId"] == frame.frame_id()),
        "requires an actual OOPIF: {targets}"
    );
    connection.close();
}

#[tokio::test]
async fn restarted_fetch_waits_for_the_body_and_a_new_quiet_window() {
    let (fixture, browser, page) = open().await;
    page.evaluate("window.body = fetch('/stream?main').then(r => r.text()); true")
        .await
        .unwrap();
    wait_for_headers(&page, "stream?main").await;
    assert_busy(&page).await;
    fixture.bodies.add_permits(1);
    assert_eq!(page.evaluate("window.body").await.unwrap(), "finished");
    assert_fresh_quiet_window(&page).await;
    browser.close().await.unwrap();
}

#[tokio::test]
async fn remote_frame_fetch_prevents_parent_network_idle() {
    let (fixture, browser, page) = open().await;
    let url = format!(
        "{}/fetch-frame.html",
        fixture.base.replace("127.0.0.1", "localhost")
    );
    let frame = open_frame(&browser, &page, &url, "child", true).await;
    wait_for_headers(&page, "stream?child").await;
    assert_busy(&page).await;
    fixture.bodies.add_permits(1);
    assert_eq!(frame.evaluate("window.body").await.unwrap(), "finished");
    assert_fresh_quiet_window(&page).await;
    browser.close().await.unwrap();
}

#[tokio::test]
async fn concurrent_requests_and_a_second_burst_restart_the_quiet_window() {
    let (fixture, browser, page) = open().await;
    page.evaluate("window.bodies = [fetch('/stream?one').then(r => r.text()), fetch('/stream?two').then(r => r.text())]; true").await.unwrap();
    wait_for_headers(&page, "stream?one").await;
    wait_for_headers(&page, "stream?two").await;
    fixture.bodies.add_permits(1);
    page.evaluate("Promise.race(window.bodies)").await.unwrap();
    assert_busy(&page).await;
    fixture.bodies.add_permits(1);
    page.evaluate("Promise.all(window.bodies)").await.unwrap();
    tokio::time::sleep(Duration::from_millis(200)).await;
    page.evaluate("window.again = fetch('/stream?again').then(r => r.text()); true")
        .await
        .unwrap();
    wait_for_headers(&page, "stream?again").await;
    assert_busy(&page).await;
    fixture.bodies.add_permits(1);
    page.evaluate("window.again").await.unwrap();
    assert_fresh_quiet_window(&page).await;
    browser.close().await.unwrap();
}

#[tokio::test]
async fn aborted_fetch_and_cancelled_idle_wait_leave_no_pending_activity() {
    let (_fixture, browser, page) = open().await;
    page.evaluate("window.controller = new AbortController(); window.body = fetch('/stream?abort', {signal: controller.signal}).then(r => r.text()).catch(() => 'aborted'); true").await.unwrap();
    wait_for_headers(&page, "stream?abort").await;
    let waiting = tokio::spawn({
        let page = page.clone();
        async move { idle(&page).await }
    });
    assert_busy(&page).await;
    waiting.abort();
    assert!(waiting.await.unwrap_err().is_cancelled());
    page.evaluate("controller.abort(); true").await.unwrap();
    assert_eq!(page.evaluate("window.body").await.unwrap(), "aborted");
    assert_fresh_quiet_window(&page).await;
    assert!(page
        .network_requests()
        .iter()
        .any(|r| r.url.contains("stream?abort") && r.failure.is_some() && r.finished.is_some()));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn removing_a_same_process_frame_releases_its_pending_requests() {
    let (fixture, browser, page) = open().await;
    let url = format!("{}/fetch-frame.html", fixture.base);
    open_frame(&browser, &page, &url, "child", false).await;
    wait_for_headers(&page, "stream?child").await;
    assert_busy(&page).await;
    page.evaluate("document.querySelector('#child').remove()")
        .await
        .unwrap();
    assert_fresh_quiet_window(&page).await;
    browser.close().await.unwrap();
}

#[tokio::test]
async fn removing_nested_renderer_frames_releases_the_entire_subtree() {
    let (fixture, browser, page) = open().await;
    let remote = fixture.base.replace("127.0.0.1", "localhost");
    let outer = open_frame(
        &browser,
        &page,
        &format!("{remote}/frame.html"),
        "outer",
        true,
    )
    .await;
    let url = format!("{}/fetch-frame.html", fixture.base);
    outer.evaluate(&format!("new Promise(resolve => {{ const f = document.createElement('iframe'); f.onload = () => resolve(true); f.src = {url:?}; document.body.append(f); }})")).await.unwrap();
    let inner = page
        .frames()
        .into_iter()
        .find(|frame| frame.url() == url)
        .unwrap();
    assert_remote(&browser, &inner).await;
    wait_for_headers(&page, "stream?child").await;
    assert_busy(&page).await;
    page.evaluate("document.querySelector('#outer').remove()")
        .await
        .unwrap();
    assert_fresh_quiet_window(&page).await;
    browser.close().await.unwrap();
}

#[tokio::test]
async fn new_main_document_does_not_wait_for_old_keepalive_or_child_requests() {
    let (fixture, browser, page) = open().await;
    page.evaluate("window.body = fetch('/stream?keepalive', {keepalive: true}).then(r => r.text()).catch(() => 'aborted'); true").await.unwrap();
    let remote = fixture.base.replace("127.0.0.1", "localhost");
    open_frame(
        &browser,
        &page,
        &format!("{remote}/fetch-frame.html"),
        "child",
        true,
    )
    .await;
    wait_for_headers(&page, "stream?keepalive").await;
    wait_for_headers(&page, "stream?child").await;
    assert_busy(&page).await;
    let started = Instant::now();
    tokio::time::timeout(
        Duration::from_secs(3),
        page.goto_with_load_state(
            &format!("{}/page.html?next", fixture.base),
            LoadState::NetworkIdle,
        ),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(started.elapsed() >= Duration::from_millis(450));
    assert!(page.url().ends_with("?next"));
    assert_eq!(page.title().await.unwrap(), "idle fixture");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn remote_frame_reload_and_return_do_not_leave_stale_pending_requests() {
    let (fixture, browser, page) = open().await;
    let remote = fixture.base.replace("127.0.0.1", "localhost");
    let frame = open_frame(
        &browser,
        &page,
        &format!("{remote}/fetch-frame.html"),
        "child",
        true,
    )
    .await;
    wait_for_headers(&page, "stream?child").await;
    frame.evaluate("location.reload(); true").await.unwrap();
    wait_for_header_count(&page, "stream?child", 2).await;
    assert_busy(&page).await;
    page.evaluate("new Promise(resolve => { const f = document.querySelector('#child'); f.onload = () => resolve(true); f.src = '/frame.html?returned'; })").await.unwrap();
    assert_fresh_quiet_window(&page).await;
    assert!(page
        .frames()
        .iter()
        .any(|f| f.url().ends_with("/frame.html?returned")));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn mocked_blocked_and_redirected_requests_finish_without_stale_counts() {
    let (fixture, browser, page) = open().await;
    page.mock("mocked", 200, "text/plain", "mock-body")
        .await
        .unwrap();
    assert_eq!(
        page.evaluate("fetch('/stream?mocked').then(r => r.text())")
            .await
            .unwrap(),
        "mock-body"
    );
    assert_fresh_quiet_window(&page).await;
    page.clear_routes().await.unwrap();
    page.block("blocked").await.unwrap();
    assert_eq!(
        page.evaluate("fetch('/stream?blocked').then(() => false, () => true)")
            .await
            .unwrap(),
        true
    );
    assert_fresh_quiet_window(&page).await;
    page.clear_routes().await.unwrap();
    page.evaluate("window.body = fetch('/redirect').then(r => r.text()); true")
        .await
        .unwrap();
    wait_for_headers(&page, "stream?redirected").await;
    assert_busy(&page).await;
    fixture.bodies.add_permits(1);
    assert_eq!(page.evaluate("window.body").await.unwrap(), "finished");
    assert_fresh_quiet_window(&page).await;
    browser.close().await.unwrap();
}

#[tokio::test]
async fn closing_during_the_quiet_timer_wakes_the_idle_waiter() {
    let (fixture, browser, page) = open().await;
    fixture.bodies.add_permits(1);
    page.evaluate("fetch('/stream?closing').then(r => r.text())")
        .await
        .unwrap();
    let result = tokio::time::timeout(Duration::from_millis(400), async {
        let (wait, close) = tokio::join!(
            page.wait_for_load_state_with_timeout(LoadState::NetworkIdle, Duration::from_secs(3)),
            async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                page.close().await
            },
        );
        close.unwrap();
        wait
    })
    .await
    .expect("closing must wake the quiet timer");
    assert!(result.unwrap_err().is_closed());
    browser.close().await.unwrap();
}
