use rustwright_test::prelude::*;

#[rustwright_test]
async fn opens_a_page(context: TestContext) -> Result<()> {
    context.page.goto("example.com").await?;
    assert!(!context.page.title().await?.is_empty());
    Ok(())
}
