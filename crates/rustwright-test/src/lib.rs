//! An independent browser test runner for Rustwright.
//!
//! This crate is intentionally separate from `rustwright-core`: the runner can
//! evolve (reporting, fixtures, retries, sharding) without coupling to the
//! automation engine. It integrates with `cargo test` by turning an annotated
//! async function into a normal `#[test]`.
//!
//! ```no_run
//! use rustwright_test::{rustwright_test, Result, TestContext};
//!
//! #[rustwright_test]
//! async fn loads_a_page(context: TestContext) -> Result<()> {
//!     context.page.goto("example.com").await?;
//!     assert!(!context.page.title().await?.is_empty());
//!     Ok(())
//! }
//! ```
//!
//! Each test gets a fresh [`Browser`], an isolated [`BrowserContext`] and a
//! [`Page`], all torn down afterwards. Tests skip (with a message) when no
//! browser is installed.
//!
//! Environment variables:
//!
//! - `RUSTWRIGHT_HEADLESS=0` runs a visible browser (default: headless).
//! - `RUSTWRIGHT_PROFILE=/path` uses a persistent profile.
//! - `RUSTWRIGHT_CHROME=/path` pins the browser executable.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::future::Future;
use std::time::Duration;

use tokio::runtime::Runtime;

pub use rustwright::prelude::{Browser, BrowserContext, Chrome, Error, Page, Result};
pub use rustwright_test_macros::rustwright_test;

/// The resources available to a test.
#[derive(Clone)]
pub struct TestContext {
    /// The browser launched for this test.
    pub browser: Browser,
    /// An isolated context owned by this test.
    pub context: BrowserContext,
    /// A page in `context`.
    pub page: Page,
}

impl TestContext {
    /// Open another page in the test's context.
    pub async fn new_page(&self) -> Result<Page> {
        self.context.new_page().await
    }

    /// Wait for a popup or new tab opened by the page.
    pub async fn wait_for_page(&self) -> Result<Page> {
        self.context.wait_for_page(Duration::from_secs(10)).await
    }
}

/// Run a browser test. Prefer the [`rustwright_test`] attribute.
///
/// This is public so the generated `#[test]` entry point can call it, and so
/// tests can be driven from a custom harness.
pub fn run_test<F, Fut>(name: &'static str, test: F)
where
    F: FnOnce(TestContext) -> Fut + Send + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    runtime().block_on(async move {
        let browser = match Browser::launch(chrome_from_env()).await {
            Ok(browser) => browser,
            Err(error) => {
                eprintln!("skipping test `{name}`: {error}");
                return;
            }
        };

        let outcome = async {
            let context = browser.new_context().await?;
            let page = context.new_page().await?;
            let test_context = TestContext {
                browser: browser.clone(),
                context: context.clone(),
                page,
            };
            let result = test(test_context).await;
            let _ = context.close().await;
            result
        }
        .await;

        let _ = browser.close().await;

        if let Err(error) = outcome {
            panic!("test `{name}` failed: {error}");
        }
    });
}

fn runtime() -> &'static Runtime {
    static RUNTIME: std::sync::OnceLock<Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::new().expect("failed to build a Tokio runtime"))
}

fn chrome_from_env() -> Chrome {
    let headless = std::env::var("RUSTWRIGHT_HEADLESS")
        .map(|value| value != "0")
        .unwrap_or(true);
    let mut chrome = Chrome::installed().headless(headless);
    if let Ok(profile) = std::env::var("RUSTWRIGHT_PROFILE") {
        chrome = chrome.profile(profile);
    }
    chrome
}
