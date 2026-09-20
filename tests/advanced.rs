//! Tests for request interception, uploads, downloads and frames.

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

/// A tiny HTTP server with an API endpoint, a download endpoint and a page.
async fn spawn_server() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").await.expect("bind");
    let address = listener.local_addr().expect("addr");
    tokio::spawn(async move {
        while let Ok((mut socket, _)) = listener.accept().await {
            let mut buffer = [0u8; 4096];
            let read = socket.read(&mut buffer).await.unwrap_or(0);
            let request = String::from_utf8_lossy(&buffer[..read]).to_string();
            let path = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/")
                .to_string();
            let (headers, body) = if path.starts_with("/download") {
                (
                    "Content-Type: text/plain\r\nContent-Disposition: attachment; filename=\"hello.txt\"\r\n",
                    "hello download".to_string(),
                )
            } else if path.starts_with("/api/data") {
                (
                    "Content-Type: application/json\r\n",
                    "{\"ok\":true}".to_string(),
                )
            } else if path.starts_with("/echo") {
                let value = request
                    .lines()
                    .find_map(|line| {
                        line.split_once(':').and_then(|(name, value)| {
                            if name.eq_ignore_ascii_case("x-rustwright") {
                                Some(value.trim().to_string())
                            } else {
                                None
                            }
                        })
                    })
                    .unwrap_or_else(|| "none".to_string());
                ("Content-Type: text/plain\r\n", value)
            } else {
                (
                    "Content-Type: text/html; charset=utf-8\r\n",
                    "<!DOCTYPE html><html><head><title>srv</title></head><body><a id=dl href=\"/download\" download>dl</a></body></html>"
                        .to_string(),
                )
            };
            let response = format!(
                "HTTP/1.1 200 OK\r\n{headers}Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = socket.write_all(response.as_bytes()).await;
            let _ = socket.shutdown().await;
        }
    });
    format!("http://{address}/")
}

async fn wait_for_file(path: &std::path::Path, timeout: Duration) -> Option<String> {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        if let Ok(contents) = std::fs::read_to_string(path) {
            return Some(contents);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    None
}

#[tokio::test]
async fn fulfills_mocked_requests() {
    if !chrome_available() {
        return;
    }
    let base = spawn_server().await;
    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch");
    let page = browser.new_page().await.expect("page");
    page.goto(&base).await.expect("goto");

    page.mock("**/api/data", 200, "application/json", r#"{"ok":true}"#)
        .await
        .expect("mock");
    let body = page
        .evaluate("fetch('/api/data').then((response) => response.text())")
        .await
        .expect("fetch mocked");
    assert_eq!(body.as_str(), Some(r#"{"ok":true}"#));
    page.clear_routes().await.expect("clear");

    browser.close().await.expect("close");
}

#[tokio::test]
async fn aborts_blocked_requests() {
    if !chrome_available() {
        return;
    }
    let base = spawn_server().await;
    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch");
    let page = browser.new_page().await.expect("page");
    page.goto(&base).await.expect("goto");

    page.block("**/api/data").await.expect("block");
    let outcome = page
        .evaluate("fetch('/api/data').then(() => 'ok').catch(() => 'blocked')")
        .await
        .expect("fetch blocked");
    assert_eq!(outcome.as_str(), Some("blocked"));

    browser.close().await.expect("close");
}

#[tokio::test]
async fn uploads_files_to_input() {
    if !chrome_available() {
        return;
    }
    let dir = std::env::temp_dir().join("rustwright-upload-tests");
    std::fs::create_dir_all(&dir).expect("dir");
    let upload = dir.join("upload.txt");
    std::fs::write(&upload, "payload").expect("write");

    let fixture = dir.join("upload.html");
    std::fs::write(
        &fixture,
        "<!DOCTYPE html><html><body><input id=\"file\" type=\"file\"></body></html>",
    )
    .expect("fixture");

    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch");
    let page = browser.new_page().await.expect("page");
    page.goto(&format!("file://{}", fixture.display()))
        .await
        .expect("goto");

    page.locator("#file")
        .set_input_files([&upload])
        .await
        .expect("set files");
    let result = page
        .evaluate("document.getElementById('file').files[0].name")
        .await
        .expect("name");
    assert_eq!(result.as_str(), Some("upload.txt"));

    browser.close().await.expect("close");
}

#[tokio::test]
async fn downloads_to_configured_path() {
    if !chrome_available() {
        return;
    }
    let base = spawn_server().await;
    let download_dir = std::env::temp_dir().join("rustwright-download-tests");
    let _ = std::fs::remove_dir_all(&download_dir);

    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch");
    browser
        .default_context()
        .set_download_path(&download_dir)
        .await
        .expect("set download path");

    let page = browser.new_page().await.expect("page");
    page.goto(&base).await.expect("goto");
    page.evaluate("document.getElementById('dl').click()")
        .await
        .expect("click download");

    let file = download_dir.join("hello.txt");
    let contents = wait_for_file(&file, Duration::from_secs(10)).await;
    assert_eq!(contents.as_deref(), Some("hello download"));

    browser.close().await.expect("close");
}

#[tokio::test]
async fn modifies_request_headers() {
    if !chrome_available() {
        return;
    }
    let base = spawn_server().await;
    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch");
    let page = browser.new_page().await.expect("page");
    page.goto(&base).await.expect("goto");

    page.route(
        "**/echo",
        RouteAction::SetRequestHeaders(vec![("x-rustwright".to_string(), "1".to_string())]),
    )
    .await
    .expect("route");
    let echoed = page
        .evaluate("fetch('/echo').then((response) => response.text())")
        .await
        .expect("fetch echo");
    assert_eq!(echoed.as_str(), Some("1"));
    page.clear_routes().await.expect("clear");

    browser.close().await.expect("close");
}

#[tokio::test]
async fn modifies_response_headers() {
    if !chrome_available() {
        return;
    }
    let base = spawn_server().await;
    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch");
    let page = browser.new_page().await.expect("page");
    page.goto(&base).await.expect("goto");

    page.route(
        "**/api/data",
        RouteAction::SetResponseHeaders(vec![("x-added".to_string(), "yes".to_string())]),
    )
    .await
    .expect("route");
    let header = page
        .evaluate("fetch('/api/data').then((response) => response.headers.get('x-added'))")
        .await
        .expect("fetch header");
    assert_eq!(header.as_str(), Some("yes"));
    page.clear_routes().await.expect("clear");

    browser.close().await.expect("close");
}

#[tokio::test]
async fn exports_har() {
    if !chrome_available() {
        return;
    }
    let base = spawn_server().await;
    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch");
    let page = browser.new_page().await.expect("page");
    page.goto(&base).await.expect("goto");
    page.evaluate("fetch('/api/data').then((response) => response.text())")
        .await
        .expect("fetch");

    let har = page.har_with_bodies().await.expect("har");
    let entries = har["log"]["entries"].as_array().expect("entries");
    let api = entries
        .iter()
        .find(|entry| {
            entry["request"]["url"]
                .as_str()
                .unwrap_or_default()
                .contains("/api/data")
        })
        .expect("api entry in HAR");
    assert_eq!(api["response"]["status"].as_i64(), Some(200));
    assert_eq!(
        api["response"]["content"]["text"].as_str(),
        Some(r#"{"ok":true}"#)
    );

    browser.close().await.expect("close");
}

#[tokio::test]
async fn records_a_chrome_trace() {
    if !chrome_available() {
        return;
    }
    let dir = std::env::temp_dir().join("rustwright-trace-tests");
    std::fs::create_dir_all(&dir).expect("dir");
    let fixture = dir.join("trace.html");
    std::fs::write(
        &fixture,
        "<!DOCTYPE html><html><head><title>trace</title></head><body><h1>trace</h1></body></html>",
    )
    .expect("fixture");

    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch");
    let page = browser.new_page().await.expect("page");
    page.goto(&format!("file://{}", fixture.display()))
        .await
        .expect("goto");

    page.start_tracing().await.expect("start tracing");
    page.evaluate("document.title = 'traced'")
        .await
        .expect("mutate");
    let trace_path = dir.join("trace.json");
    page.stop_tracing(&trace_path).await.expect("stop tracing");

    let contents = std::fs::read_to_string(&trace_path).expect("trace file");
    let document: serde_json::Value = serde_json::from_str(&contents).expect("valid json");
    let events = document["traceEvents"].as_array().expect("traceEvents");
    assert!(!events.is_empty(), "trace contains events");

    browser.close().await.expect("close");
}

#[tokio::test]
async fn interacts_with_cross_origin_iframe() {
    if !chrome_available() {
        return;
    }
    let dir = std::env::temp_dir().join("rustwright-frame-tests");
    std::fs::create_dir_all(&dir).expect("dir");
    let fixture = dir.join("frames.html");
    let html = "<!DOCTYPE html><html><head><title>frames</title></head><body>\
        <iframe src=\"data:text/html,<input id=inner placeholder=Inside>\" width=300 height=200></iframe>\
        </body></html>";
    std::fs::write(&fixture, html).expect("fixture");

    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch");
    let page = browser.new_page().await.expect("page");
    page.goto(&format!("file://{}", fixture.display()))
        .await
        .expect("goto");

    assert!(page.frames().len() >= 2, "main frame plus iframe");

    let frame = page
        .frame_locator("iframe")
        .resolve()
        .await
        .expect("resolve frame");
    frame
        .get_by_placeholder("Inside")
        .fill("from-frame")
        .await
        .expect("fill in frame");

    let value = frame
        .evaluate("document.getElementById('inner').value")
        .await
        .expect("read in frame");
    assert_eq!(value.as_str(), Some("from-frame"));

    browser.close().await.expect("close");
}
