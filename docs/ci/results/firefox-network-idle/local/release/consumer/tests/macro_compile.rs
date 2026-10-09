use rustwright_test::{Result, TestContext};
#[rustwright_test::rustwright_test]
async fn reexported_macro(_context: TestContext) -> Result<()> { Ok(()) }
#[rustwright_test_macros::rustwright_test]
async fn direct_macro(_context: TestContext) -> Result<()> { Ok(()) }
