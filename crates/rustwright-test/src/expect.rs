//! Playwright-style `expect(locator)` assertions.
//!
//! Assertion failures panic (like `assert!`), while transport errors propagate
//! as `Result`, so a test body reads naturally:
//!
//! ```no_run
//! use rustwright_test::prelude::*;
//!
//! # async fn run(context: TestContext) -> Result<()> {
//! expect(context.page.get_by_role(Role::Button, Some("Submit")))
//!     .to_be_visible()
//!     .await?;
//! expect(context.page.locator("h1")).to_have_text("Welcome").await?;
//! # Ok(())
//! # }
//! ```

use crate::{AnyLocator, LocatorApi, Result};

/// An assertion target built from an [`AnyLocator`].
pub struct Expectation {
    locator: AnyLocator,
}

/// Start an assertion on a locator.
pub fn expect(locator: AnyLocator) -> Expectation {
    Expectation { locator }
}

impl Expectation {
    /// Assert the element exists and is visible.
    pub async fn to_be_visible(&self) -> Result<()> {
        if self.locator.is_visible().await? {
            Ok(())
        } else {
            panic!("expected {} to be visible", self.locator.describe())
        }
    }

    /// Assert the element is absent or not visible.
    pub async fn to_be_hidden(&self) -> Result<()> {
        if self.locator.is_hidden().await? {
            Ok(())
        } else {
            panic!("expected {} to be hidden", self.locator.describe())
        }
    }

    /// Assert the element is enabled.
    pub async fn to_be_enabled(&self) -> Result<()> {
        if self.locator.is_enabled().await? {
            Ok(())
        } else {
            panic!("expected {} to be enabled", self.locator.describe())
        }
    }

    /// Assert the element's text equals `expected`.
    pub async fn to_have_text(&self, expected: &str) -> Result<()> {
        let actual = self.locator.text().await?;
        if actual == expected {
            Ok(())
        } else {
            panic!(
                "expected {} to have text {expected:?}, got {actual:?}",
                self.locator.describe()
            )
        }
    }

    /// Assert the element's text contains `expected`.
    pub async fn to_contain_text(&self, expected: &str) -> Result<()> {
        let actual = self.locator.text().await?;
        if actual.contains(expected) {
            Ok(())
        } else {
            panic!(
                "expected {} to contain text {expected:?}, got {actual:?}",
                self.locator.describe()
            )
        }
    }

    /// Assert the number of matching elements equals `expected`.
    pub async fn to_have_count(&self, expected: usize) -> Result<()> {
        let actual = self.locator.count().await?;
        if actual == expected {
            Ok(())
        } else {
            panic!(
                "expected {} to have count {expected}, got {actual}",
                self.locator.describe()
            )
        }
    }

    /// Assert an attribute equals `expected`.
    pub async fn to_have_attribute(&self, name: &str, expected: &str) -> Result<()> {
        let actual = self.locator.get_attribute(name).await?;
        if actual.as_deref() == Some(expected) {
            Ok(())
        } else {
            panic!(
                "expected {} to have attribute {name:?}={expected:?}, got {actual:?}",
                self.locator.describe()
            )
        }
    }
}
