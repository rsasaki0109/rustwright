//! Tests for locator collections, semantic locators, forms, input, dialogs,
//! viewport, scrolling and storage state.
//!
//! All fixtures are local, so no network access is required.

use std::path::PathBuf;
use std::time::Duration;

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
    let dir = std::env::temp_dir().join("rustwright-feature-tests");
    std::fs::create_dir_all(&dir).expect("create fixture dir");
    let path: PathBuf = dir.join(format!("{name}.html"));
    std::fs::write(&path, html).expect("write fixture");
    format!("file://{}", path.display())
}

const FEATURES_HTML: &str = r#"<!DOCTYPE html>
<html>
  <head><title>Features</title></head>
  <body>
    <input placeholder="Search" />
    <label for="email">Email</label>
    <input id="email" />
    <button data-testid="go" onclick="document.title='go-clicked';">Go</button>
    <ul>
      <li class="item">A</li>
      <li class="item">B</li>
      <li class="item">C</li>
    </ul>
    <input type="checkbox" id="cb" />
    <select id="sel">
      <option value="a">A</option>
      <option value="b">B</option>
    </select>
    <button id="alert" onclick="alert('hello')">Alert</button>
    <button id="hover" title="hover me">Hover</button>
    <div style="height: 3000px;">tall</div>
    <button id="bottom">Bottom</button>
  </body>
</html>"#;

async fn launch() -> Browser {
    Browser::launch(Chrome::installed().headless(true))
        .await
        .expect("launch")
}

#[tokio::test]
async fn locator_collections() {
    if !chrome_available() {
        return;
    }
    let url = fixture("collections", FEATURES_HTML);
    let browser = launch().await;
    let page = browser.new_page().await.expect("page");
    page.goto(&url).await.expect("goto");

    let items = page.locator("li.item");
    assert_eq!(items.count().await.expect("count"), 3);
    assert_eq!(items.first().text().await.expect("first"), "A");
    assert_eq!(items.last().text().await.expect("last"), "C");
    assert_eq!(items.nth(1).text().await.expect("nth"), "B");
    let all = items.all().await.expect("all");
    assert_eq!(all.len(), 3);
    assert_eq!(all[2].text().await.expect("all[2]"), "C");
    assert_eq!(items.nth(99).count().await.expect("out of range"), 0);

    browser.close().await.expect("close");
}

#[tokio::test]
async fn semantic_locators() {
    if !chrome_available() {
        return;
    }
    let url = fixture("semantic", FEATURES_HTML);
    let browser = launch().await;
    let page = browser.new_page().await.expect("page");
    page.goto(&url).await.expect("goto");

    page.get_by_placeholder("Search")
        .fill("rust")
        .await
        .expect("placeholder fill");
    let placeholder_value = page
        .evaluate("document.querySelector('input[placeholder=Search]').value")
        .await
        .expect("value");
    assert_eq!(placeholder_value.as_str(), Some("rust"));

    page.get_by_label("Email")
        .fill("a@b.c")
        .await
        .expect("label fill");
    let email = page
        .evaluate("document.getElementById('email').value")
        .await
        .expect("email");
    assert_eq!(email.as_str(), Some("a@b.c"));

    page.get_by_test_id("go")
        .click()
        .await
        .expect("test id click");
    assert_eq!(page.title().await.expect("title"), "go-clicked");

    browser.close().await.expect("close");
}

#[tokio::test]
async fn check_and_select() {
    if !chrome_available() {
        return;
    }
    let url = fixture("forms", FEATURES_HTML);
    let browser = launch().await;
    let page = browser.new_page().await.expect("page");
    page.goto(&url).await.expect("goto");

    let checkbox = page.locator("#cb");
    assert!(!checkbox.is_checked().await.expect("unchecked"));
    checkbox.check().await.expect("check");
    assert!(checkbox.is_checked().await.expect("checked"));
    checkbox.uncheck().await.expect("uncheck");
    assert!(!checkbox.is_checked().await.expect("unchecked again"));

    page.locator("#sel")
        .select_option("b")
        .await
        .expect("select");
    let selected = page
        .evaluate("document.getElementById('sel').value")
        .await
        .expect("value");
    assert_eq!(selected.as_str(), Some("b"));

    browser.close().await.expect("close");
}

#[tokio::test]
async fn hover_scroll_and_wheel() {
    if !chrome_available() {
        return;
    }
    let url = fixture("scroll", FEATURES_HTML);
    let browser = launch().await;
    let page = browser.new_page().await.expect("page");
    page.goto(&url).await.expect("goto");

    page.locator("#hover").hover().await.expect("hover");

    let bottom = page.locator("#bottom");
    bottom
        .scroll_into_view_if_needed()
        .await
        .expect("scroll into view");
    let at_bottom = page.evaluate("window.scrollY").await.expect("scrollY");
    assert!(at_bottom.as_f64().unwrap_or(0.0) > 0.0);

    page.evaluate("window.scrollTo(0, 0)").await.expect("reset");
    page.scroll_by(0.0, 600.0).await.expect("wheel");
    let scrolled = page.evaluate("window.scrollY").await.expect("scrollY");
    assert!(scrolled.as_f64().unwrap_or(0.0) > 0.0);

    browser.close().await.expect("close");
}

#[tokio::test]
async fn auto_dismisses_dialogs() {
    if !chrome_available() {
        return;
    }
    let url = fixture("dialog", FEATURES_HTML);
    let browser = launch().await;
    let page = browser.new_page().await.expect("page");
    page.goto(&url).await.expect("goto");

    page.locator("#alert").click().await.expect("alert click");

    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while page.dialogs().is_empty() && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let dialogs = page.dialogs();
    assert_eq!(dialogs.len(), 1, "dialog captured");
    assert_eq!(dialogs[0].message, "hello");

    // The page is still responsive after auto-dismissal.
    let title = page.title().await.expect("title after dialog");
    assert_eq!(title, "Features");

    browser.close().await.expect("close");
}

#[tokio::test]
async fn viewport_is_configurable() {
    if !chrome_available() {
        return;
    }
    let url = fixture("viewport", FEATURES_HTML);
    let browser = launch().await;
    let page = browser.new_page().await.expect("page");
    page.set_viewport(&Viewport::new(800, 600))
        .await
        .expect("set viewport");
    page.goto(&url).await.expect("goto");

    let width = page
        .evaluate("window.innerWidth")
        .await
        .expect("innerWidth");
    assert_eq!(width.as_i64(), Some(800));

    browser.close().await.expect("close");
}

#[tokio::test]
async fn history_and_wait_for_url() {
    if !chrome_available() {
        return;
    }
    let first = fixture(
        "history-one",
        "<!DOCTYPE html><title>one</title><body>1</body>",
    );
    let second = fixture(
        "history-two",
        "<!DOCTYPE html><title>two</title><body>2</body>",
    );
    let browser = launch().await;
    let page = browser.new_page().await.expect("page");
    page.goto(&first).await.expect("goto first");
    page.goto(&second).await.expect("goto second");
    page.wait_for_url("history-two").await.expect("wait url");

    page.go_back().await.expect("go back");
    assert!(page.url().contains("history-one"), "url: {}", page.url());

    browser.close().await.expect("close");
}

#[tokio::test]
async fn storage_state_round_trips_local_storage() {
    if !chrome_available() {
        return;
    }
    let url = fixture("storage", FEATURES_HTML);
    let browser = launch().await;
    let page = browser.new_page().await.expect("page");
    page.goto(&url).await.expect("goto");

    page.evaluate("localStorage.setItem('token', 'abc')")
        .await
        .expect("set item");
    let state = page.storage_state().await.expect("storage state");
    assert!(state.origins.iter().any(|origin| origin
        .local_storage
        .iter()
        .any(|item| item.name == "token" && item.value == "abc")));

    page.evaluate("localStorage.clear()").await.expect("clear");
    page.restore_storage_state(&state)
        .await
        .expect("restore state");
    let restored = page
        .evaluate("localStorage.getItem('token')")
        .await
        .expect("get item");
    assert_eq!(restored.as_str(), Some("abc"));

    browser.close().await.expect("close");
}
