//! Open a real, logged-in browsing session and keep it open for manual login.
//!
//! Rustwright uses an installed, unmodified Chrome with a persistent profile, so
//! you can log in to Mercari, Rakuma or any SNS by hand and then reuse the same
//! profile from other Rustwright programs (cookies and storage persist).
//!
//! ```sh
//! cargo run -p rustwright-examples --example browse_session -- --profile ./target/live https://x.com/
//! ```
//!
//! Log in manually in the window, then press Ctrl+C to close gracefully.

use std::path::PathBuf;

use rustwright::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let mut url = "about:blank".to_string();
    let mut profile = PathBuf::from("./target/rustwright-profile");
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--profile" => {
                if let Some(value) = args.next() {
                    profile = PathBuf::from(value);
                }
            }
            other => url = other.to_string(),
        }
    }

    let browser = Browser::launch(Chrome::installed().headless(false).profile(&profile)).await?;
    println!("browser: {}", browser.version().browser);
    println!("profile: {}", profile.display());

    let page = browser.new_page().await?;
    if let Err(error) = page.goto(&url).await {
        eprintln!("navigation warning: {error}");
    }
    println!("opened: {}", page.url());
    println!("log in manually, then press Ctrl+C to close.");

    let _ = tokio::signal::ctrl_c().await;
    println!("\nclosing...");
    browser.close().await?;
    Ok(())
}
