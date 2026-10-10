//! Navigation regressions served over local HTTP, without external services.

use std::{sync::Arc, time::Duration};

use rustwright::prelude::*;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Notify,
    task::{JoinHandle, JoinSet},
};

struct Fixture {
    base: String,
    image: Arc<Notify>,
    image_started: Arc<Notify>,
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
    let image = Arc::new(Notify::new());
    let image_gate = image.clone();
    let image_started = Arc::new(Notify::new());
    let image_start = image_started.clone();
    let task = tokio::spawn(async move {
        let mut requests = JoinSet::new();
        loop {
            tokio::select! {
                accepted = listener.accept() => {
                    let (mut socket, _) = accepted.unwrap();
                    let image = image_gate.clone();
                    let image_started = image_start.clone();
                    requests.spawn(async move {
                        let mut bytes = Vec::new();
                        loop {
                            let mut buffer = [0; 2048];
                            let Ok(count) = socket.read(&mut buffer).await else { return; };
                            if count == 0 { return; }
                            bytes.extend_from_slice(&buffer[..count]);
                            if bytes.windows(4).any(|part| part == b"\r\n\r\n") { break; }
                        }
                        let request = String::from_utf8_lossy(&bytes);
                        let path = request.lines().next().unwrap().split_whitespace().nth(1).unwrap().split('?').next().unwrap();
                        let (status, headers, body) = match path {
                            "/ready.html" => ("200 OK", "", "<!DOCTYPE html><title>ready</title><a id=next href=/loading.html>next</a>"),
                            "/loading.html" => ("200 OK", "", "<!DOCTYPE html><title>loading</title><img src=/held.svg>"),
                            "/held.svg" => {
                                image_started.notify_one();
                                image.notified().await;
                                ("200 OK", "", "<svg xmlns='http://www.w3.org/2000/svg'/>")
                            },
                            "/redirect-one" => ("302 Found", "Location: /redirect-two\r\n", ""),
                            "/redirect-two" => ("307 Temporary Redirect", "Location: /ready.html?redirected\r\n", ""),
                            "/delayed.html" => {
                                tokio::time::sleep(Duration::from_millis(180)).await;
                                ("200 OK", "", "<!DOCTYPE html><title>delayed</title><img src=/delayed.svg>")
                            },
                            "/delayed.svg" => {
                                tokio::time::sleep(Duration::from_millis(180)).await;
                                ("200 OK", "", "<svg xmlns='http://www.w3.org/2000/svg'/>")
                            },
                            _ => ("404 Not Found", "", ""),
                        };
                        let mime = if path.ends_with(".svg") { "image/svg+xml" } else { "text/html" };
                        let response = format!("HTTP/1.1 {status}\r\n{headers}Content-Type: {mime}\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                        let _ = socket.write_all(response.as_bytes()).await;
                    });
                },
                _ = requests.join_next(), if !requests.is_empty() => {},
            }
        }
    });
    let fixture = Fixture {
        base,
        image,
        image_started,
        task,
    };
    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .unwrap();
    let page = browser.new_page().await.unwrap();
    page.goto(&format!("{}/ready.html", fixture.base))
        .await
        .unwrap();
    (fixture, browser, page)
}

#[tokio::test]
async fn fragment_navigation_preserves_the_document_load_state() {
    let (fixture, browser, page) = open().await;
    let url = format!("{}/ready.html#anchor", fixture.base);
    page.goto_with_timeout(&url, Duration::from_millis(200))
        .await
        .expect("same-document navigation does not emit another load");
    assert_eq!(page.url(), url);
    assert_eq!(page.main_frame().url(), url);
    browser.close().await.unwrap();
}

#[tokio::test]
async fn url_wait_observes_push_state_and_hash_changes() {
    let (_fixture, browser, page) = open().await;
    for expression in [
        "history.pushState({}, '', '#pushed')",
        "location.hash = 'hashed'",
    ] {
        page.evaluate(expression).await.unwrap();
        let fragment = if expression.starts_with("history") {
            "#pushed"
        } else {
            "#hashed"
        };
        page.wait_for_url_with_timeout(fragment, Duration::from_millis(200))
            .await
            .unwrap();
        assert_eq!(page.main_frame().url(), page.url());
    }
    browser.close().await.unwrap();
}

#[tokio::test]
async fn redirected_goto_reports_the_committed_url() {
    let (fixture, browser, page) = open().await;
    page.goto(&format!("{}/redirect-one", fixture.base))
        .await
        .unwrap();
    assert_eq!(
        page.url(),
        format!("{}/ready.html?redirected", fixture.base)
    );
    assert_eq!(page.main_frame().url(), page.url());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn click_navigation_does_not_reuse_the_previous_documents_load() {
    let (fixture, browser, page) = open().await;
    let next = page.locator("#next");
    let (wait, click) = tokio::join!(
        page.wait_for_url_with_timeout("/loading.html", Duration::from_secs(2)),
        next.click(),
    );
    wait.unwrap();
    click.unwrap();
    let result = page
        .wait_for_load_state_with_timeout(LoadState::Load, Duration::from_millis(100))
        .await;
    assert!(
        matches!(result, Err(Error::Timeout { .. })),
        "image is still blocked: {result:?}"
    );
    page.wait_for_load_state_with_timeout(LoadState::DomContentLoaded, Duration::from_secs(1))
        .await
        .unwrap();
    fixture.image.notify_one();
    page.wait_for_load_state_with_timeout(LoadState::Load, Duration::from_secs(1))
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn goto_timeout_covers_headers_and_document_load_together() {
    let (fixture, browser, page) = open().await;
    let result = page
        .goto_with_timeout(
            &format!("{}/delayed.html", fixture.base),
            Duration::from_millis(280),
        )
        .await;
    assert!(
        matches!(result, Err(Error::Timeout { .. })),
        "180ms headers + 180ms image exceed one 280ms budget: {result:?}"
    );
    page.goto_with_timeout(
        &format!("{}/ready.html?recovered", fixture.base),
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    assert_eq!(page.title().await.unwrap(), "ready");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn closing_a_page_wakes_pending_url_waiters() {
    let (_fixture, browser, page) = open().await;
    let result = tokio::time::timeout(Duration::from_millis(500), async {
        let (wait, close) = tokio::join!(
            page.wait_for_url_with_timeout("never-matches", Duration::from_secs(5)),
            async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                page.close().await
            },
        );
        close.unwrap();
        wait
    })
    .await
    .expect("page close must wake the waiter");
    assert!(result.unwrap_err().is_closed());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn same_document_history_and_full_document_history_finish() {
    let (fixture, browser, page) = open().await;
    for fragment in ["#one", "#two"] {
        page.goto_with_timeout(
            &format!("{}/ready.html{fragment}", fixture.base),
            Duration::from_secs(1),
        )
        .await
        .unwrap();
    }
    tokio::time::timeout(Duration::from_secs(2), page.go_back())
        .await
        .unwrap()
        .unwrap();
    assert!(page.url().ends_with("#one"), "{}", page.url());
    tokio::time::timeout(Duration::from_secs(2), page.go_forward())
        .await
        .unwrap()
        .unwrap();
    assert!(page.url().ends_with("#two"));
    page.goto(&format!("{}/ready.html?other-document", fixture.base))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), page.go_back())
        .await
        .unwrap()
        .unwrap();
    assert!(page.url().ends_with("#two"), "{}", page.url());
    tokio::time::timeout(Duration::from_secs(2), page.go_forward())
        .await
        .unwrap()
        .unwrap();
    assert!(page.url().ends_with("?other-document"));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn reload_waits_for_the_new_document_and_its_resources() {
    let (fixture, browser, page) = open().await;
    fixture.image.notify_one();
    page.goto(&format!("{}/loading.html", fixture.base))
        .await
        .unwrap();
    fixture.image_started.notified().await;
    let mut reloading = tokio::spawn({
        let page = page.clone();
        async move { page.reload().await }
    });
    tokio::time::timeout(Duration::from_secs(2), fixture.image_started.notified())
        .await
        .unwrap();
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut reloading)
            .await
            .is_err(),
        "reload must wait for its resource"
    );
    fixture.image.notify_one();
    tokio::time::timeout(Duration::from_secs(2), reloading)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(page.title().await.unwrap(), "loading");
    browser.close().await.unwrap();
}

#[tokio::test]
async fn cancelled_navigation_does_not_corrupt_the_next_navigation() {
    let (fixture, browser, page) = open().await;
    let navigating = tokio::spawn({
        let page = page.clone();
        let url = format!("{}/loading.html", fixture.base);
        async move { page.goto(&url).await }
    });
    tokio::time::timeout(Duration::from_secs(2), fixture.image_started.notified())
        .await
        .unwrap();
    navigating.abort();
    assert!(navigating.await.unwrap_err().is_cancelled());
    page.goto_with_timeout(
        &format!("{}/ready.html?after-cancel", fixture.base),
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    page.wait_for_load_state_with_timeout(LoadState::Load, Duration::from_millis(100))
        .await
        .unwrap();
    assert!(page.url().ends_with("?after-cancel"));
    browser.close().await.unwrap();
}

#[tokio::test]
async fn another_document_interrupts_an_older_navigation_waiter() {
    let (fixture, browser, page) = open().await;
    let navigating = tokio::spawn({
        let page = page.clone();
        let url = format!("{}/loading.html", fixture.base);
        async move { page.goto(&url).await }
    });
    tokio::time::timeout(Duration::from_secs(2), fixture.image_started.notified())
        .await
        .unwrap();
    page.goto_with_timeout(
        &format!("{}/ready.html?newer", fixture.base),
        Duration::from_secs(2),
    )
    .await
    .unwrap();
    let result = tokio::time::timeout(Duration::from_secs(1), navigating)
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(result, Err(Error::Navigation(_))),
        "the newer document's load cannot complete the older navigation: {result:?}"
    );
    browser.close().await.unwrap();
}

#[tokio::test]
async fn closing_a_page_wakes_pending_load_waiters() {
    let (fixture, browser, page) = open().await;
    page.goto_with_load_state(
        &format!("{}/loading.html", fixture.base),
        LoadState::DomContentLoaded,
    )
    .await
    .unwrap();
    let result = tokio::time::timeout(Duration::from_millis(500), async {
        let (wait, close) = tokio::join!(
            page.wait_for_load_state_with_timeout(LoadState::Load, Duration::from_secs(5)),
            async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                page.close().await
            },
        );
        close.unwrap();
        wait
    })
    .await
    .expect("page close must wake the load waiter");
    assert!(result.unwrap_err().is_closed());
    browser.close().await.unwrap();
}

#[tokio::test]
async fn browser_disconnect_wakes_pending_waiters() {
    let (_fixture, browser, page) = open().await;
    let result = tokio::time::timeout(Duration::from_secs(2), async {
        let (wait, close) = tokio::join!(
            page.wait_for_url_with_timeout("never-matches", Duration::from_secs(10)),
            async {
                tokio::time::sleep(Duration::from_millis(20)).await;
                browser.close().await
            },
        );
        close.unwrap();
        wait
    })
    .await
    .expect("disconnect must wake the waiter");
    assert!(result.unwrap_err().is_closed());
}

#[tokio::test]
async fn url_wait_observes_a_matching_url_even_if_it_changes_again_immediately() {
    let (_fixture, browser, page) = open().await;
    let (wait, change) = tokio::join!(
        page.wait_for_url_with_timeout("#wanted", Duration::from_millis(300)),
        page.evaluate(
            "history.pushState({}, '', '#wanted'); history.replaceState({}, '', '#later'); true"
        ),
    );
    change.unwrap();
    wait.expect("the matching URL event must not be lost when updates coalesce");
    page.wait_for_url_with_timeout("#later", Duration::from_millis(300))
        .await
        .unwrap();
    browser.close().await.unwrap();
}

#[tokio::test]
async fn timeout_before_commit_allows_a_new_navigation_and_ignores_the_old_response() {
    let (fixture, browser, page) = open().await;
    let result = page
        .goto_with_timeout(
            &format!("{}/delayed.html", fixture.base),
            Duration::from_millis(90),
        )
        .await;
    assert!(matches!(result, Err(Error::Timeout { .. })), "{result:?}");
    let url = format!("{}/ready.html?after-command-timeout", fixture.base);
    page.goto_with_timeout(&url, Duration::from_secs(2))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(220)).await;
    assert_eq!(page.url(), url);
    assert_eq!(page.title().await.unwrap(), "ready");
    browser.close().await.unwrap();
}
