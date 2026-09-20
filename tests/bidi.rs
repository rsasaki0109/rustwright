//! Tests for the WebDriver BiDi + Firefox path.
//!
//! These skip cleanly when Firefox is not installed.

use std::time::Duration;

use rustwright::bidi::{BidiBrowser, BidiResult};
use rustwright::prelude::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn firefox_available() -> bool {
    match Firefox::installed().executable_path() {
        Ok(path) => {
            eprintln!("using firefox: {}", path.display());
            true
        }
        Err(error) => {
            eprintln!("skipping test, no firefox found: {error}");
            false
        }
    }
}

/// Launch Firefox, or return `None` (skip) when it cannot run here.
///
/// CI images sometimes ship a Firefox that is installed but not runnable (for
/// example a Snap without a session bus), so launch failures skip rather than
/// fail the suite.
async fn launch_firefox() -> Option<BidiBrowser> {
    match BidiBrowser::launch(Firefox::installed().headless(true)).await {
        Ok(browser) => Some(browser),
        Err(error) => {
            eprintln!("skipping test, firefox launch failed: {error}");
            None
        }
    }
}

/// A minimal HTTP server for network diagnostics and interception.
async fn spawn_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buffer = [0u8; 2048];
            let read = socket.read(&mut buffer).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/")
                .to_string();
            let (content_type, body) = if path.starts_with("/api/data") {
                ("application/json", r#"{"real":true}"#.to_string())
            } else {
                (
                    "text/html",
                    "<!DOCTYPE html><html><head><title>net</title></head><body>ok</body></html>"
                        .to_string(),
                )
            };
            let cookie = if path.starts_with("/nocookie") {
                ""
            } else {
                "Set-Cookie: rw=1; Path=/\r\n"
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\n{cookie}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    format!("http://{address}/")
}

#[tokio::test]
async fn firefox_bidi_navigates_and_evaluates() -> BidiResult<()> {
    if !firefox_available() {
        return Ok(());
    }
    // A data URL avoids filesystem access, which sandboxed Firefox builds
    // (for example the Snap) cannot do for arbitrary paths.
    let url = "data:text/html,<title>bidi</title><h1>Hello BiDi</h1>";

    let Some(browser) = launch_firefox().await else {
        return Ok(());
    };
    assert!(browser.browser_version().is_some());

    let page = browser.new_page().await?;
    page.goto(url).await?;
    assert_eq!(page.title().await?, "bidi");
    assert!(page.url().await?.starts_with("data:"));

    let heading = page
        .evaluate("document.querySelector('h1').textContent")
        .await?;
    assert_eq!(heading.as_str(), Some("Hello BiDi"));

    let content = page.content().await?;
    assert!(content.contains("Hello BiDi"));

    let screenshot = std::env::temp_dir()
        .join("rustwright-bidi-tests")
        .join("bidi.png");
    page.screenshot(&screenshot).await?;
    assert!(std::fs::read(&screenshot).expect("screenshot").len() > 8);

    browser.close().await?;
    Ok(())
}

#[tokio::test]
async fn firefox_bidi_locators_interact() -> BidiResult<()> {
    if !firefox_available() {
        return Ok(());
    }
    let url = "data:text/html,<title>locators</title>\
        <input name='q' placeholder='Search'>\
        <button id='go'>Go</button>\
        <div id='out'>waiting</div>";

    let Some(browser) = launch_firefox().await else {
        return Ok(());
    };
    let page = browser.new_page().await?;
    page.goto(url).await?;

    // Arrange a click handler without relying on inline scripts in the URL.
    page.evaluate(
        "document.getElementById('go').addEventListener('click', () => { \
           document.getElementById('out').textContent = 'clicked'; })",
    )
    .await?;

    let search = page.locator("input[name=q]");
    assert!(search.is_visible().await?);
    search.fill("rust").await?;
    let value = page
        .evaluate("document.querySelector('input[name=q]').value")
        .await?;
    assert_eq!(value.as_str(), Some("rust"));

    assert_eq!(page.locator("button").count().await?, 1);

    page.get_by_role(Role::Button, Some("Go")).click().await?;
    let output = page.get_by_text("clicked");
    output.wait_for(WaitState::Visible).await?;
    assert_eq!(output.text().await?, "clicked");

    assert_eq!(page.locator("#missing").count().await?, 0);
    assert!(page.locator("#missing").is_hidden().await?);

    browser.close().await?;
    Ok(())
}

#[tokio::test]
async fn firefox_bidi_network_monitoring() -> BidiResult<()> {
    if !firefox_available() {
        return Ok(());
    }
    let base = spawn_server().await;
    let Some(browser) = launch_firefox().await else {
        return Ok(());
    };
    let page = browser.new_page().await?;
    page.start_network_monitoring().await?;
    page.goto(&base).await?;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while page.network_requests().is_empty() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    let requests = page.network_requests();
    assert!(
        requests.iter().any(|request| request.url == base),
        "document request was captured, got: {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request.url == base && request.status == Some(200)),
        "response status was captured"
    );

    browser.close().await?;
    Ok(())
}

#[tokio::test]
async fn firefox_bidi_frames() -> BidiResult<()> {
    if !firefox_available() {
        return Ok(());
    }
    let Some(browser) = launch_firefox().await else {
        return Ok(());
    };
    let page = browser.new_page().await?;
    let url = "data:text/html,<title>frames</title>\
        <iframe src='data:text/html,<input id=inner placeholder=Inside>'></iframe>";
    page.goto(url).await?;

    let frames = page.frames().await?;
    assert_eq!(frames.len(), 1, "iframe became a child frame: {frames:?}");

    let frame = page.frame_locator("iframe").await?;
    frame
        .get_by_placeholder("Inside")
        .fill("from-frame")
        .await?;
    let value = frame
        .evaluate("document.getElementById('inner').value")
        .await?;
    assert_eq!(value.as_str(), Some("from-frame"));

    browser.close().await?;
    Ok(())
}

#[tokio::test]
async fn firefox_bidi_cookies_and_storage() -> BidiResult<()> {
    if !firefox_available() {
        return Ok(());
    }
    let base = spawn_server().await;
    let Some(browser) = launch_firefox().await else {
        return Ok(());
    };
    let page = browser.new_page().await?;
    page.goto(&base).await?;

    let cookies = page.cookies().await?;
    assert!(
        cookies
            .iter()
            .any(|cookie| cookie.name == "rw" && cookie.value == "1"),
        "server cookie was captured: {cookies:?}"
    );

    page.add_cookie("added", "yes", "127.0.0.1").await?;
    let cookies = page.cookies().await?;
    assert!(
        cookies
            .iter()
            .any(|cookie| cookie.name == "added" && cookie.value == "yes"),
        "cookie was added: {cookies:?}"
    );

    page.evaluate("localStorage.setItem('token', 'abc')")
        .await?;
    let state = page.storage_state().await?;
    assert!(state.origins.iter().any(|origin| origin
        .local_storage
        .iter()
        .any(|item| item.name == "token" && item.value == "abc")));

    page.evaluate("localStorage.clear()").await?;
    page.restore_storage_state(&state).await?;
    let restored = page.evaluate("localStorage.getItem('token')").await?;
    assert_eq!(restored.as_str(), Some("abc"));

    browser.close().await?;
    Ok(())
}

#[tokio::test]
async fn firefox_bidi_isolated_user_context() -> BidiResult<()> {
    if !firefox_available() {
        return Ok(());
    }
    let base = spawn_server().await;
    let Some(browser) = launch_firefox().await else {
        return Ok(());
    };

    let default_page = browser.new_page().await?;
    default_page.goto(&base).await?;
    default_page
        .evaluate("localStorage.setItem('k', 'default')")
        .await?;
    assert_eq!(
        default_page
            .evaluate("localStorage.getItem('k')")
            .await?
            .as_str(),
        Some("default")
    );

    let context = browser.new_context().await?;
    let isolated = context.new_page().await?;
    isolated.goto(&base).await?;
    assert!(
        isolated
            .evaluate("localStorage.getItem('k')")
            .await?
            .is_null(),
        "isolated context must not see the default context's local storage"
    );
    assert_eq!(context.pages().await?.len(), 1);

    context.close().await?;
    browser.close().await?;
    Ok(())
}

#[tokio::test]
async fn firefox_bidi_request_interception() -> BidiResult<()> {
    if !firefox_available() {
        return Ok(());
    }
    let base = spawn_server().await;
    let Some(browser) = launch_firefox().await else {
        return Ok(());
    };
    let page = browser.new_page().await?;

    page.mock("**/api/data", 200, "application/json", r#"{"ok":true}"#)
        .await?;
    page.goto(&base).await?;
    let body = page
        .evaluate("fetch('/api/data').then((response) => response.text())")
        .await?;
    assert_eq!(body.as_str(), Some(r#"{"ok":true}"#));
    page.clear_routes().await?;

    page.block("**/api/data").await?;
    let outcome = page
        .evaluate("fetch('/api/data').then(() => 'ok').catch(() => 'blocked')")
        .await?;
    assert_eq!(outcome.as_str(), Some("blocked"));

    browser.close().await?;
    Ok(())
}
