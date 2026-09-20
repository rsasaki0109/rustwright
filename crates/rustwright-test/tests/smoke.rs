//! Smoke tests for the `#[rustwright_test]` runner.

use rustwright_test::prelude::*;

#[rustwright_test]
async fn opens_a_data_url(context: TestContext) -> Result<()> {
    context
        .page
        .goto("data:text/html,<title>runner</title><h1>Hello</h1>")
        .await?;
    assert_eq!(context.page.title().await?, "runner");
    assert!(context.page.locator("h1").is_visible().await?);
    Ok(())
}

#[rustwright_test]
async fn supports_extra_pages(context: TestContext) -> Result<()> {
    context.page.goto("about:blank").await?;
    let _other = context.new_page().await?;
    assert!(context.pages().await?.len() >= 2);
    Ok(())
}
