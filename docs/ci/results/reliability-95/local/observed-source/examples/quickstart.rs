//! The Quick Start from the README.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p rustwright-examples --example quickstart
//! cargo run -p rustwright-examples --example quickstart -- --headless https://example.com/
//! ```

use rustwright::prelude::*;

#[path = "support/run_options.rs"]
mod run_options;

#[tokio::main]
async fn main() -> Result<()> {
    let Some(options) = run_options::RunOptions::parse("quickstart", "page.png")? else {
        return Ok(());
    };
    let mut chrome = Chrome::installed().headless(options.headless);
    if let Some(profile) = options.profile {
        chrome = chrome.profile(profile);
    }
    let browser = Browser::launch(chrome).await?;
    println!("connected to {}", browser.version().browser);

    let page = browser.new_page().await?;
    page.goto(&options.url).await?;
    println!("title: {}", page.title().await?);

    let heading = page.locator("h1");
    println!("h1: {}", heading.text().await?);

    page.screenshot(&options.screenshot).await?;
    println!("screenshot: {}", options.screenshot.display());

    browser.close().await?;
    Ok(())
}
