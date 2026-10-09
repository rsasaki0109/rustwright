//! Drive Firefox over WebDriver BiDi.
//!
//! This uses the BiDi transport (separate from the CDP path used for Chrome).
//!
//! ```sh
//! cargo run -p rustwright-examples --example bidi_firefox
//! cargo run -p rustwright-examples --example bidi_firefox -- --headless --profile ./target/firefox-example https://example.com/
//! ```

use rustwright::bidi::{BidiBrowser, BidiResult};
use rustwright::prelude::*;

#[path = "support/run_options.rs"]
mod run_options;

#[tokio::main]
async fn main() -> BidiResult<()> {
    let Some(options) = run_options::RunOptions::parse("bidi_firefox", "firefox.png")? else {
        return Ok(());
    };
    let mut firefox = Firefox::installed().headless(options.headless);
    if let Some(profile) = options.profile {
        firefox = firefox.profile(profile);
    }
    let browser = BidiBrowser::launch(firefox).await?;
    println!("firefox: {:?}", browser.browser_version());

    let page = browser.new_page().await?;
    page.goto(&options.url).await?;
    println!("title: {}", page.title().await?);

    let heading = page
        .evaluate("document.querySelector('h1') && document.querySelector('h1').textContent")
        .await?;
    println!("h1: {heading}");

    page.screenshot(&options.screenshot).await?;
    println!("screenshot: {}", options.screenshot.display());

    browser.close().await?;
    Ok(())
}
