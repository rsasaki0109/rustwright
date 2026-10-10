#![allow(dead_code, unused_variables)]
use rustwright::prelude::*;
use std::time::Duration;
use rustwright_test::expect;
pub async fn api_sketch() -> rustwright::Result<()> {
// Launch, headless or headed.
let browser = Browser::launch(Chrome::installed().headless(false)).await?;

// Persistent profile.
let browser = Browser::launch(
    Chrome::installed().profile("./profile").headless(false)
).await?;

// Or attach to a running browser:
//   google-chrome --remote-debugging-port=9222 --user-data-dir=/tmp/live
let browser = Browser::connect("http://127.0.0.1:9222").await?;

// Pages and contexts.
let page = browser.new_page().await?;
let context = browser.new_context().await?;   // isolated cookies/storage

// Navigation.
page.goto("https://example.com").await?;
page.reload().await?;
println!("{}", page.url());
println!("{}", page.title().await?);
let html = page.content().await?;
page.screenshot("page.png").await?;

// Locators (lazy + auto-waiting).
let q = page.locator("input[name=q]");
q.fill("hello").await?;
q.click_with_timeout(Duration::from_secs(2)).await?;
println!("{}", q.text().await?);
println!("{}", q.is_visible().await?);

// Locator collections.
let items = page.locator("li.item");
println!("{}", items.count().await?);
items.first().click().await?;
items.nth(2).hover().await?;
for item in items.all().await? { println!("{}", item.text().await?); }

// Semantic locators.
page.get_by_text("Login").click().await?;
page.get_by_role(Role::Button, Some("Submit")).click().await?;
page.get_by_placeholder("Search").fill("rust").await?;
page.get_by_label("Email").fill("a@b.c").await?;
page.get_by_test_id("submit").click().await?;

// Forms and real input.
page.locator("#agree").check().await?;
page.locator("#plan").select_option("pro").await?;
page.locator("#search").press("Enter").await?;
page.mouse_wheel(400.0, 300.0, 0.0, 800.0).await?; // infinite-scroll feeds

// Waiting.
page.wait_for_load_state(LoadState::DomContentLoaded).await?;
page.wait_for_load_state(LoadState::NetworkIdle).await?;
page.wait_for_url("/dashboard").await?;
page.locator("#result").wait_for(WaitState::Visible).await?;

// Popups / new tabs, and reusable login state.
let popup = browser.default_context().wait_for_page(Duration::from_secs(10)).await?;
let state = page.storage_state().await?;       // cookies + localStorage
page.restore_storage_state(&state).await?;

// Request interception (generic; no site-specific rules).
page.mock("**/api/data", 200, "application/json", r#"{"ok":true}"#).await?;
page.block("**/ads/**").await?;
page.route("**/echo", RouteAction::SetRequestHeaders(vec![("x-app".into(), "1".into())])).await?;
page.route("**/api/**", RouteAction::SetResponseHeaders(vec![("x-served-by".into(), "rustwright".into())])).await?;
page.clear_routes().await?;

// Frames, including cross-origin iframes.
let frame = page.frame_locator("iframe").resolve().await?;
frame.get_by_placeholder("Search").fill("rust").await?;
println!("{}", frame.url());

// Uploads and downloads.
browser.default_context().set_download_path("./downloads").await?;
page.locator("input[type=file]").set_input_files(["./a.pdf"]).await?;

// A stable viewport for deterministic SPA rendering.
page.set_viewport(&Viewport::new(1280, 800)).await?;

// Chrome tracing (JSON with optional screenshots).
page.start_tracing().await?;
// ... exercise the page ...
page.stop_tracing("trace.json").await?;

// Diagnostics.
for message in page.console_messages() { println!("[{}] {}", message.level, message.text); }
for error in page.errors() { println!("error: {}", error.message); }
for dialog in page.dialogs() { println!("dialog: {}", dialog.message); }
let har = page.har_with_bodies().await?;   // HAR 1.2 network log
let diag = browser.diagnostics();
println!("{:?}", diag.launch_args);

Ok(())
}
pub async fn dynamic_pages(browser: &Browser, bidi_browser: &BidiBrowser) -> std::result::Result<(), AnyError> {
let mut pages: Vec<AnyPage> = Vec::new();
pages.push(browser.new_page().await?.into());          // Chrome
pages.push(bidi_browser.new_page().await?.into());     // Firefox
for page in &pages {
    page.goto("example.com").await?;
    println!("[{}] {}", page.backend_name(), page.title().await?);
}

Ok(())
}
pub async fn assertions(page: &AnyPage) -> rustwright_test::Result<()> {
expect(page.locator("h1")).to_have_text("Welcome").await?;
expect(page.get_by_role(Role::Button, Some("Submit"))).to_be_visible().await?;
expect(page.locator("li.item")).to_have_count(3).await?;

expect(page.locator("li.item"))
    .with_timeout(std::time::Duration::from_secs(2))
    .to_have_count(3)
    .await?;

Ok(())
}
pub fn browser_configuration() {
let chrome = Chrome::at("/usr/bin/google-chrome-stable");
let chrome = Chrome::installed().variant(ChromeVariant::Chromium);
// or: RUSTWRIGHT_CHROME=/path/to/chrome

}
mod runner {
use rustwright_test::prelude::*;

#[rustwright_test]
async fn opens_a_page(context: TestContext) -> Result<()> {
    context.page.goto("example.com").await?;
    assert!(!context.page.title().await?.is_empty());
    Ok(())
}

}
