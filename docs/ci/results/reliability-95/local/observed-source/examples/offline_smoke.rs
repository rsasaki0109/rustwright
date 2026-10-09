//! An offline end-to-end walk-through, served over loopback HTTP.
//!
//! Launch -> new page -> goto -> locators -> trusted click/Unicode fill -> screenshot.
//! The embedded page needs no internet access or external HTTP server.
//!
//! ```sh
//! cargo run -p rustwright-examples --example offline_smoke
//! cargo run -p rustwright-examples --example offline_smoke -- --screenshot ./smoke.png
//! ```

use std::{io, path::PathBuf, time::Duration};

use rustwright::prelude::*;
use serde_json::json;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    task::{JoinHandle, JoinSet},
};

const HTML: &str = r#"<!DOCTYPE html>
<html>
  <head><meta charset="utf-8"><title>Rustwright</title></head>
  <body>
    <h1>Hello, Rustwright</h1>
    <input name="q" placeholder="Search" />
    <script>window.clickCount = 0; window.lastTrusted = false;</script>
    <button id="go" onclick="window.clickCount++; window.lastTrusted = event.isTrusted; document.getElementById('out').textContent = 'clicked';">Go</button>
    <div id="out">waiting</div>
  </body>
</html>"#;

const INPUT: &str = "Rustwright 日本語 🦀";

#[tokio::main]
async fn main() -> Result<()> {
    let Some(screenshot) = screenshot_path()? else {
        return Ok(());
    };
    let fixture = Fixture::start().await?;
    let browser = Browser::launch(Chrome::installed().headless(true)).await?;
    println!("browser: {}", browser.version().browser);
    println!("fixture: {}", fixture.url);

    let outcome: Result<()> = async {
        let page = browser.new_page().await?;
        page.goto(&fixture.url).await?;
        let title = page.title().await?;
        ensure(title == "Rustwright", "unexpected fixture title")?;
        println!("title: {title}");

        page.locator("input[name=q]").fill(INPUT).await?;
        let value = page
            .evaluate("document.querySelector('input[name=q]').value")
            .await?;
        ensure(value == json!(INPUT), "Unicode input did not round-trip")?;
        println!("input value: {value}");

        page.get_by_role(Role::Button, Some("Go")).click().await?;
        let output = page.get_by_text("clicked");
        output.wait_for(WaitState::Visible).await?;
        ensure(
            output.text().await? == "clicked",
            "click output did not update",
        )?;
        let click = page
            .evaluate("[window.clickCount, window.lastTrusted]")
            .await?;
        ensure(
            click == json!([1, true]),
            "expected exactly one trusted click",
        )?;
        println!("click: {click}");

        page.screenshot(&screenshot).await?;
        ensure(
            std::fs::read(&screenshot)?.starts_with(b"\x89PNG\r\n\x1a\n"),
            "screenshot is not a PNG",
        )?;
        println!("screenshot: {}", screenshot.display());
        ensure(page.errors().is_empty(), "fixture reported page errors")?;
        println!("page errors: {}", page.errors().len());
        Ok(())
    }
    .await;

    let closed = browser.close().await;
    outcome?;
    closed
}

fn ensure(condition: bool, message: &str) -> io::Result<()> {
    if condition {
        Ok(())
    } else {
        Err(io::Error::other(message))
    }
}

fn screenshot_path() -> io::Result<Option<PathBuf>> {
    let mut screenshot =
        std::env::temp_dir().join(format!("rustwright-smoke-{}.png", std::process::id()));
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--help" | "-h" => {
                println!("Usage: offline_smoke [--screenshot PATH]\nRuns headless using an embedded loopback HTTP fixture.");
                return Ok(None);
            }
            "--screenshot" => {
                screenshot = args
                    .next()
                    .filter(|path| !path.is_empty() && !path.starts_with('-'))
                    .map(PathBuf::from)
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "--screenshot requires a path")
                    })?;
            }
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown argument: {other}"),
                ));
            }
        }
    }
    Ok(Some(screenshot))
}

struct Fixture {
    url: String,
    task: JoinHandle<()>,
}

impl Fixture {
    async fn start() -> io::Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/", listener.local_addr()?);
        let task = tokio::spawn(async move {
            let mut requests = JoinSet::new();
            loop {
                tokio::select! {
                    incoming = listener.accept() => {
                        let Ok((mut socket, _)) = incoming else { break };
                        requests.spawn(async move {
                            let _ = tokio::time::timeout(Duration::from_secs(5), async {
                                let mut request = Vec::new();
                                let mut buffer = [0; 1024];
                                while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
                                    let count = socket.read(&mut buffer).await?;
                                    if count == 0 || request.len() + count > 16 * 1024 {
                                        return Ok::<_, io::Error>(());
                                    }
                                    request.extend_from_slice(&buffer[..count]);
                                }
                                let headers = format!(
                                    "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                    HTML.len()
                                );
                                socket.write_all(headers.as_bytes()).await?;
                                socket.write_all(HTML.as_bytes()).await?;
                                socket.shutdown().await
                            }).await;
                        });
                    }
                    _ = requests.join_next(), if !requests.is_empty() => {}
                }
            }
        });
        Ok(Self { url, task })
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
