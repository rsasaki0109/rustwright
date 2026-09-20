//! The Quick Start from the README.
//!
//! Run with:
//!
//! ```sh
//! cargo run -p rustwright-examples --example quickstart
//! ```

use rustwright::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let browser = Browser::launch(Chrome::installed().headless(false)).await?;
    println!("connected to {}", browser.version().browser);

    let page = browser.new_page().await?;
    page.goto("example.com").await?;
    println!("title: {}", page.title().await?);

    let heading = page.locator("h1");
    println!("h1: {}", heading.text().await?);

    page.screenshot("page.png").await?;

    browser.close().await?;
    Ok(())
}
