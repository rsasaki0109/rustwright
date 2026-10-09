//! Playwright-style `expect(locator)` assertions.
//!
//! Assertions retry until they match, with a default timeout of five seconds.
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

use std::fmt::Debug;
use std::future::Future;
use std::time::Duration;

use tokio::time::{sleep_until, timeout_at, Instant};

use crate::{AnyError, AnyLocator, LocatorApi, Result};

const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);
const RETRY_INTERVAL: Duration = Duration::from_millis(50);

/// An assertion target built from an [`AnyLocator`].
///
/// Matchers retry for up to five seconds by default. Use
/// [`Self::with_timeout`] to change the deadline for this target.
pub struct Expectation {
    locator: AnyLocator,
    timeout: Duration,
}

/// Start an assertion on a locator.
pub fn expect(locator: AnyLocator) -> Expectation {
    Expectation {
        locator,
        timeout: DEFAULT_TIMEOUT,
    }
}

impl Expectation {
    /// Set the maximum time each matcher waits, including locator reads.
    ///
    /// ```no_run
    /// use std::time::Duration;
    /// use rustwright_test::prelude::*;
    ///
    /// # async fn check(context: TestContext) -> Result<()> {
    /// expect(context.page.locator("li"))
    ///     .with_timeout(Duration::from_secs(2))
    ///     .to_have_count(3)
    ///     .await?;
    /// # Ok(())
    /// # }
    /// ```
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Wait for the element to exist and be visible.
    pub async fn to_be_visible(&self) -> Result<()> {
        self.retry(
            "to be visible",
            || self.locator.is_visible(),
            |actual| *actual,
        )
        .await
    }

    /// Wait for the element to be absent or not visible.
    pub async fn to_be_hidden(&self) -> Result<()> {
        self.retry(
            "to be hidden",
            || self.locator.is_hidden(),
            |actual| *actual,
        )
        .await
    }

    /// Wait for the element to be enabled.
    pub async fn to_be_enabled(&self) -> Result<()> {
        self.retry(
            "to be enabled",
            || self.locator.is_enabled(),
            |actual| *actual,
        )
        .await
    }

    /// Wait for the element's text to equal `expected`.
    pub async fn to_have_text(&self, expected: &str) -> Result<()> {
        self.retry(
            &format!("to have text {expected:?}"),
            || self.locator.text(),
            |actual| actual == expected,
        )
        .await
    }

    /// Wait for the element's text to contain `expected`.
    pub async fn to_contain_text(&self, expected: &str) -> Result<()> {
        self.retry(
            &format!("to contain text {expected:?}"),
            || self.locator.text(),
            |actual| actual.contains(expected),
        )
        .await
    }

    /// Wait for the number of matching elements to equal `expected`.
    pub async fn to_have_count(&self, expected: usize) -> Result<()> {
        self.retry(
            &format!("to have count {expected}"),
            || self.locator.count(),
            |actual| *actual == expected,
        )
        .await
    }

    /// Wait for an attribute to equal `expected`.
    pub async fn to_have_attribute(&self, name: &str, expected: &str) -> Result<()> {
        self.retry(
            &format!("to have attribute {name:?}={expected:?}"),
            || self.locator.get_attribute(name),
            |actual| actual.as_deref() == Some(expected),
        )
        .await
    }

    async fn retry<T, F, Fut, P>(&self, expected: &str, mut observe: F, matches: P) -> Result<()>
    where
        T: Debug,
        F: FnMut() -> Fut,
        Fut: Future<Output = Result<T>>,
        P: Fn(&T) -> bool,
    {
        let deadline = Instant::now() + self.timeout;
        let mut last_observation = "no completed observation".to_string();
        loop {
            // Locator reads may have their own longer timeout. Bound them by
            // the assertion's deadline as well as bounding the retry interval.
            match timeout_at(deadline, observe()).await {
                Ok(Ok(actual)) => {
                    if matches(&actual) {
                        return Ok(());
                    }
                    last_observation = format!("got {actual:?}");
                }
                Ok(Err(error)) if is_missing_element(&error) => {
                    last_observation = error.to_string();
                }
                Ok(Err(error)) => return Err(error),
                Err(_) => break,
            }
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            sleep_until((now + RETRY_INTERVAL).min(deadline)).await;
            if Instant::now() >= deadline {
                break;
            }
        }
        panic!(
            "expected {} {expected}, timed out after {:?}; {last_observation}",
            self.locator.describe(),
            self.timeout,
        );
    }
}

// DOM replacement can remove an element between waiting for it and reading it.
// Retry locator absence and frame readiness; preserve transport and JavaScript errors.
fn is_missing_element(error: &AnyError) -> bool {
    matches!(
        error,
        AnyError::Chrome(
            rustwright::Error::ElementNotFound { .. }
                | rustwright::Error::Timeout { .. }
                | rustwright::Error::FrameNotReady { .. }
        ) | AnyError::Firefox(
            rustwright::BidiError::ElementNotFound(_) | rustwright::BidiError::WaitTimeout(_)
        )
    )
}
