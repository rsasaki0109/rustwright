use rustwright::bidi::BidiBrowser;
use rustwright::prelude::*;

#[tokio::main]
async fn main() -> rustwright::BidiResult<()> {
    let mut firefox = Firefox::installed().headless(true);
    if let Some(profile) = std::env::var_os("RUSTWRIGHT_PROFILE") {
        firefox = firefox.profile(profile);
    }
    let browser = BidiBrowser::launch(firefox).await?;
    let page = browser.new_page().await?;
    let url = std::env::args().nth(1).unwrap_or_else(|| "https://example.com".into());
    page.goto(&url).await?;
    println!("{}", page.title().await?);

    // example.com has a heading; form interaction is shown by offline_smoke.
    println!("{}", page.locator("h1").text().await?);
    page.screenshot("firefox.png").await?;

    browser.close().await?;
    Ok(())
}
