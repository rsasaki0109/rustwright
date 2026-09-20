//! Launch Chrome with a persistent profile.
//!
//! Cookies, local/session storage and logins survive between runs because the
//! same `--user-data-dir` is reused. This example needs network access for
//! `example.com`.
//!
//! ```sh
//! cargo run -p rustwright-examples --example persistent_profile
//! ```

use rustwright::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let profile = std::path::PathBuf::from("./target/rustwright-profile");

    let browser = Browser::launch(Chrome::installed().headless(true).profile(&profile)).await?;

    let diagnostics = browser.diagnostics();
    println!("executable: {:?}", diagnostics.executable);
    println!("user data dir: {:?}", diagnostics.user_data_dir);
    println!("launch args: {:?}", diagnostics.launch_args);

    let page = browser.new_page().await?;
    page.goto("https://example.com").await?;

    // Stores a value in local storage under the persistent profile.
    page.evaluate("localStorage.setItem('rustwright', 'hello')")
        .await?;
    let stored = page.evaluate("localStorage.getItem('rustwright')").await?;
    println!("stored: {stored}");

    browser.close().await?;
    Ok(())
}
