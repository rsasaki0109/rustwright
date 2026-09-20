//! Drive Firefox over WebDriver BiDi.
//!
//! This uses the BiDi transport (separate from the CDP path used for Chrome).
//!
//! ```sh
//! cargo run -p rustwright-examples --example bidi_firefox
//! ```

use rustwright::bidi::{BidiBrowser, BidiResult};
use rustwright::prelude::*;

#[tokio::main]
async fn main() -> BidiResult<()> {
    let browser = BidiBrowser::launch(Firefox::installed().headless(false)).await?;
    println!("firefox: {:?}", browser.browser_version());

    let page = browser.new_page().await?;
    page.goto("https://example.com/").await?;
    println!("title: {}", page.title().await?);

    let heading = page
        .evaluate("document.querySelector('h1') && document.querySelector('h1').textContent")
        .await?;
    println!("h1: {heading}");

    page.screenshot("firefox.png").await?;
    println!("screenshot: firefox.png");

    browser.close().await?;
    Ok(())
}
