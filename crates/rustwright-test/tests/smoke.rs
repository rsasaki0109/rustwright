//! Smoke tests for the `#[rustwright_test]` runner.

use rustwright_test::{rustwright_test, Result, TestContext};

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
async fn provides_an_isolated_context_with_extra_pages(context: TestContext) -> Result<()> {
    context.page.goto("about:blank").await?;
    let other = context.new_page().await?;
    assert!(!other.is_closed());
    assert_eq!(context.context.pages().len(), 2);
    Ok(())
}
