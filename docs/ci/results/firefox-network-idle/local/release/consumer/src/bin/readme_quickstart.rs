use rustwright::prelude::*;

#[tokio::main]
async fn main() -> Result<()> {
    let browser = Browser::launch(
        Chrome::installed().headless(true)
    ).await?;
    let page = browser.new_page().await?;
    let url = std::env::args().nth(1).unwrap_or_else(|| "https://example.com".into());
    page.goto(&url).await?;
    println!("{}", page.title().await?);
    browser.close().await?;
    Ok(())
}
