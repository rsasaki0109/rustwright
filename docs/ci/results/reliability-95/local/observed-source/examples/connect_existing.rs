//! Connect to an already-running Chrome.
//!
//! Start Chrome yourself first, for example:
//!
//! ```sh
//! google-chrome --remote-debugging-port=9222 --user-data-dir=/tmp/rustwright-live
//! ```
//!
//! then run:
//!
//! ```sh
//! cargo run -p rustwright-examples --example connect_existing
//! ```

use rustwright::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let browser = Browser::connect("http://127.0.0.1:9222").await?;
    println!("connected to {}", browser.version().browser);

    // Attach to tabs that are already open in the user's session.
    let pages = browser.pages().await?;
    println!("open pages: {}", pages.len());
    for page in pages {
        println!("  {}", page.url());
    }

    if let Some(page) = browser.pages().await?.into_iter().next() {
        println!("title: {}", page.title().await?);
    }

    // For a connected browser, close only disconnects; the user's browser stays open.
    browser.close().await?;
    Ok(())
}
