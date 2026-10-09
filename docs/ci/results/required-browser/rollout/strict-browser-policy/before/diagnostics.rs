//! Diagnostics tests: the observability that makes environment deltas debuggable.

use std::path::PathBuf;
use std::time::Duration;

use rustwright::prelude::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

fn chrome_available() -> bool {
    match Chrome::installed().executable_path() {
        Ok(path) => {
            eprintln!("using browser: {}", path.display());
            true
        }
        Err(error) => {
            eprintln!("skipping test, no browser found: {error}");
            false
        }
    }
}

fn fixture(name: &str, html: &str) -> String {
    let dir = std::env::temp_dir().join("rustwright-tests");
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let path: PathBuf = dir.join(format!("{name}.html"));
    std::fs::write(&path, html).expect("write fixture");
    format!("file://{}", path.display())
}

async fn wait_until(timeout: Duration, mut predicate: impl FnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if predicate() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// A minimal one-request HTTP server used to exercise cookies and network
/// diagnostics without touching the network.
async fn spawn_test_server(body: &'static str) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("local addr");
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buffer = [0u8; 2048];
            let _ = socket.read(&mut buffer).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n\
                 Set-Cookie: rw=1; Path=/\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    format!("http://{address}/")
}

#[tokio::test]
async fn expose_launch_diagnostics() {
    if !chrome_available() {
        return;
    }
    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch");
    let diagnostics = browser.diagnostics();
    assert!(diagnostics.executable.is_some());
    assert!(diagnostics.user_data_dir.is_some());
    assert!(diagnostics.ephemeral_profile);
    assert!(diagnostics.pid.is_some());
    assert!(diagnostics.endpoint.is_some());
    assert!(diagnostics
        .launch_args
        .iter()
        .any(|arg| arg.starts_with("--remote-debugging-port")));
    assert!(diagnostics
        .launch_args
        .iter()
        .any(|arg| arg == "--headless=new"));

    browser.close().await.expect("close");
}

#[tokio::test]
async fn capture_console_messages_and_errors() {
    if !chrome_available() {
        return;
    }
    let url = fixture(
        "diagnostics",
        "<!DOCTYPE html><html><head><title>diag</title></head><body>ok</body></html>",
    );
    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch");
    let page = browser.new_page().await.expect("new page");
    page.goto(&url).await.expect("goto");

    page.evaluate("console.log('hello diagnostics')")
        .await
        .expect("console log");
    let saw_console = wait_until(Duration::from_secs(5), || {
        page.console_messages()
            .iter()
            .any(|message| message.text.contains("hello diagnostics"))
    })
    .await;
    assert!(saw_console, "console message was captured");

    page.evaluate("setTimeout(() => { throw new Error('boom'); }, 0)")
        .await
        .expect("schedule error");
    let saw_error = wait_until(Duration::from_secs(5), || {
        page.errors()
            .iter()
            .any(|error| error.message.contains("boom"))
    })
    .await;
    assert!(saw_error, "page error was captured");

    browser.close().await.expect("close");
}

#[tokio::test]
async fn expose_cookies_navigations_and_network() {
    if !chrome_available() {
        return;
    }
    let url = spawn_test_server(
        "<!DOCTYPE html><html><head><title>server</title></head><body>hi</body></html>",
    )
    .await;
    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch");
    let page = browser.new_page().await.expect("new page");
    page.goto(&url).await.expect("goto");

    let navigations = page.navigations();
    assert!(
        navigations.iter().any(|navigation| navigation.url == url),
        "navigation was recorded, got: {navigations:?}"
    );

    let cookies = page.cookies().await.expect("cookies");
    assert!(
        cookies
            .iter()
            .any(|cookie| cookie.name == "rw" && cookie.value == "1"),
        "cookie was captured, got: {cookies:?}"
    );

    let requests = page.network_requests();
    assert!(
        requests
            .iter()
            .any(|request| request.url == url && request.status == Some(200)),
        "document request with status was captured, got: {requests:?}"
    );

    browser.close().await.expect("close");
}
