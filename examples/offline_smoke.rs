//! A fully offline end-to-end walk-through of the MVP vertical slice.
//!
//! Launch -> new page -> goto -> locators -> click/fill -> wait -> screenshot.
//! It uses a local file URL, so no network access is required.
//!
//! ```sh
//! cargo run -p rustwright-examples --example offline_smoke
//! ```

use std::path::PathBuf;

use rustwright::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let html = r#"<!DOCTYPE html>
<html>
  <head><title>Rustwright</title></head>
  <body>
    <h1>Hello, Rustwright</h1>
    <input name="q" placeholder="Search" />
    <button id="go" onclick="document.getElementById('out').textContent = 'clicked';">Go</button>
    <div id="out">waiting</div>
  </body>
</html>"#;

    let html_path: PathBuf = std::env::temp_dir().join("rustwright-smoke.html");
    std::fs::write(&html_path, html)?;
    let url = format!("file://{}", html_path.display());

    let browser = Browser::launch(Chrome::installed().headless(true)).await?;
    println!("browser: {}", browser.version().browser);

    let page = browser.new_page().await?;
    page.goto(&url).await?;
    println!("title: {}", page.title().await?);

    let search = page.locator("input[name=q]");
    search.fill("rust").await?;
    let value = page
        .evaluate("document.querySelector('input[name=q]').value")
        .await?;
    println!("input value: {value}");

    page.get_by_role(Role::Button, Some("Go")).click().await?;
    let output = page.get_by_text("clicked");
    output.wait_for(WaitState::Visible).await?;
    println!("output: {}", output.text().await?);

    let screenshot = std::env::temp_dir().join("rustwright-smoke.png");
    page.screenshot(&screenshot).await?;
    println!("screenshot: {}", screenshot.display());

    println!("console messages: {}", page.console_messages().len());
    println!("page errors: {}", page.errors().len());

    browser.close().await?;
    Ok(())
}
