//! End-to-end tests for the MVP vertical slice.
//!
//! These run against the Chrome/Chromium installed on the machine and skip
//! cleanly when none is found. They intentionally avoid the network by using
//! local fixtures.

use std::path::PathBuf;

use rustwright::prelude::*;

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

const FIXTURE_HTML: &str = r#"<!DOCTYPE html>
<html>
  <head><title>Rustwright Fixture</title></head>
  <body>
    <h1>Hello, Rustwright</h1>
    <input name="q" placeholder="Search" />
    <button id="go" onclick="document.getElementById('out').textContent = 'clicked';">Go</button>
    <div id="out">waiting</div>
  </body>
</html>"#;

#[tokio::test]
async fn launch_goto_title_and_content() {
    if !chrome_available() {
        return;
    }
    let url = fixture("basic", FIXTURE_HTML);
    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch browser");
    let page = browser.new_page().await.expect("new page");
    page.goto(&url).await.expect("goto");

    assert_eq!(page.title().await.expect("title"), "Rustwright Fixture");
    let content = page.content().await.expect("content");
    assert!(content.contains("Hello, Rustwright"));
    assert!(content.to_lowercase().starts_with("<!doctype html"));
    assert!(page.url().starts_with("file://"));

    browser.close().await.expect("close");
}

#[tokio::test]
async fn locators_fill_click_and_wait() {
    if !chrome_available() {
        return;
    }
    let url = fixture("locators", FIXTURE_HTML);
    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch");
    let page = browser.new_page().await.expect("new page");
    page.goto(&url).await.expect("goto");

    let search = page.locator("input[name=q]");
    assert!(search.is_visible().await.expect("is_visible"));
    search.fill("rust").await.expect("fill");

    let value = page
        .evaluate("document.querySelector('input[name=q]').value")
        .await
        .expect("evaluate");
    assert_eq!(value.as_str(), Some("rust"));

    page.get_by_role(Role::Button, Some("Go"))
        .click()
        .await
        .expect("role click");

    let output = page.get_by_text("clicked");
    output
        .wait_for(WaitState::Visible)
        .await
        .expect("wait for output");
    assert_eq!(output.text().await.expect("text"), "clicked");
    assert!(output.is_visible().await.expect("visible"));

    browser.close().await.expect("close");
}

#[tokio::test]
async fn wait_for_detached_and_load_states() {
    if !chrome_available() {
        return;
    }
    let url = fixture("wait", FIXTURE_HTML);
    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch");
    let page = browser.new_page().await.expect("new page");
    page.goto(&url).await.expect("goto");
    page.wait_for_load_state(LoadState::DomContentLoaded)
        .await
        .expect("domcontentloaded");
    page.wait_for_load_state(LoadState::NetworkIdle)
        .await
        .expect("networkidle");

    let missing = page.locator("#does-not-exist");
    assert_eq!(missing.count().await.expect("count"), 0);
    assert!(missing.is_hidden().await.expect("hidden"));
    missing
        .wait_for_with_timeout(WaitState::Detached, std::time::Duration::from_secs(5))
        .await
        .expect("detached");

    browser.close().await.expect("close");
}

#[tokio::test]
async fn screenshot_writes_a_png() {
    if !chrome_available() {
        return;
    }
    let url = fixture("screenshot", FIXTURE_HTML);
    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch");
    let page = browser.new_page().await.expect("new page");
    page.goto(&url).await.expect("goto");

    let path = std::env::temp_dir()
        .join("rustwright-tests")
        .join("shot.png");
    page.screenshot(&path).await.expect("screenshot");
    let bytes = std::fs::read(&path).expect("read screenshot");
    assert!(bytes.len() > 8);
    assert_eq!(&bytes[1..4], b"PNG");

    browser.close().await.expect("close");
}

#[tokio::test]
async fn isolated_context_has_separate_storage() {
    if !chrome_available() {
        return;
    }
    let url = fixture("context", FIXTURE_HTML);
    let browser = Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch");

    let page = browser.new_page().await.expect("new page");
    page.goto(&url).await.expect("goto");
    page.evaluate("localStorage.setItem('key', 'default')")
        .await
        .expect("set default");

    let context = browser.new_context().await.expect("new context");
    let isolated = context.new_page().await.expect("isolated page");
    isolated.goto(&url).await.expect("goto isolated");
    let value = isolated
        .evaluate("localStorage.getItem('key')")
        .await
        .expect("read isolated");
    assert!(value.is_null());

    context.close().await.expect("close context");
    browser.close().await.expect("close browser");
}
