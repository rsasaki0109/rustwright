//! An independent browser test runner for Rustwright.
//!
//! This crate is intentionally separate from `rustwright-core`: the runner can
//! evolve (reporting, fixtures, retries, sharding) without coupling to the
//! automation engine. It integrates with `cargo test` by turning an annotated
//! async function into a normal `#[test]`.
//!
//! ```no_run
//! use rustwright_test::prelude::*;
//!
//! #[rustwright_test]
//! async fn loads_a_page(context: TestContext) -> Result<()> {
//!     context.page.goto("example.com").await?;
//!     assert!(!context.page.title().await?.is_empty());
//!     Ok(())
//! }
//! ```
//!
//! Each test gets a fresh browser, an isolated context and a page, all torn
//! down afterwards. Tests run against Chrome (CDP) by default; set
//! `RUSTWRIGHT_BROWSER=firefox` to run the same tests against Firefox over
//! WebDriver BiDi. Tests skip (with a message) when no browser is installed.
//!
//! Environment variables:
//!
//! - `RUSTWRIGHT_BROWSER=firefox` runs against Firefox/BiDi (default: Chrome).
//! - `RUSTWRIGHT_HEADLESS=0` runs a visible browser (default: headless).
//! - `RUSTWRIGHT_PROFILE=/path` uses a persistent profile.
//! - `RUSTWRIGHT_CHROME` / `RUSTWRIGHT_FIREFOX` pin the executable.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::future::Future;

use tokio::runtime::Runtime;

pub use rustwright::{
    AnyError, AnyLocator, AnyPage, Browser, BrowserContext, Chrome, Firefox, LocatorApi, PageApi,
    Role, Selector, WaitState,
};
pub use rustwright_test_macros::rustwright_test;

/// The result type used by Rustwright tests.
///
/// It is the dynamic backend error ([`AnyError`]), so a test can drive either
/// Chrome or Firefox without caring which one is selected.
pub type Result<T> = std::result::Result<T, AnyError>;

/// The types a test usually needs.
pub mod prelude {
    pub use crate::{rustwright_test, LocatorApi, PageApi, Result, TestContext};
}

/// The resources available to a test.
pub struct TestContext {
    backend: Backend,
    /// The page the test starts with.
    pub page: AnyPage,
}

enum Backend {
    Chrome {
        _browser: Browser,
        context: BrowserContext,
    },
    Firefox {
        _browser: rustwright::BidiBrowser,
    },
}

impl TestContext {
    /// The backend in use: `"chrome"` or `"firefox"`.
    pub fn browser_name(&self) -> &'static str {
        match self.backend {
            Backend::Chrome { .. } => "chrome",
            Backend::Firefox { .. } => "firefox",
        }
    }

    /// Open another page (a new tab) for this test.
    pub async fn new_page(&self) -> Result<AnyPage> {
        match &self.backend {
            Backend::Chrome { context, .. } => Ok(context.new_page().await?.into()),
            Backend::Firefox { _browser: browser } => Ok(browser.new_page().await?.into()),
        }
    }

    /// All pages (tabs) currently open in the test's browser session.
    pub async fn pages(&self) -> Result<Vec<AnyPage>> {
        match &self.backend {
            Backend::Chrome { context, .. } => Ok(context
                .refresh_pages()
                .await?
                .into_iter()
                .map(AnyPage::from)
                .collect()),
            Backend::Firefox { _browser: browser } => Ok(browser
                .pages()
                .await?
                .into_iter()
                .map(AnyPage::from)
                .collect()),
        }
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
        let headless = std::env::var("RUSTWRIGHT_HEADLESS")
            .map(|value| value != "0")
            .unwrap_or(true);
        let backend = std::env::var("RUSTWRIGHT_BROWSER").unwrap_or_default();

        let context = match backend.as_str() {
            "firefox" | "bidi" => match launch_firefox(headless).await {
                Ok(context) => context,
                Err(error) => {
                    eprintln!("skipping test `{name}`: firefox unavailable: {error}");
                    return;
                }
            },
            _ => match launch_chrome(headless).await {
                Ok(context) => context,
                Err(error) => {
                    eprintln!("skipping test `{name}`: chrome unavailable: {error}");
                    return;
                }
            },
        };

        if let Err(error) = test(context).await {
            panic!("test `{name}` failed: {error}");
        }
    });
}

async fn launch_chrome(headless: bool) -> Result<TestContext> {
    let browser = Browser::launch(chrome_from_env(headless)).await?;
    let context = browser.new_context().await?;
    let page = context.new_page().await?;
    Ok(TestContext {
        backend: Backend::Chrome {
            _browser: browser,
            context,
        },
        page: page.into(),
    })
}

async fn launch_firefox(headless: bool) -> Result<TestContext> {
    let browser = rustwright::BidiBrowser::launch(firefox_from_env(headless)).await?;
    let page = browser.new_page().await?;
    Ok(TestContext {
        backend: Backend::Firefox { _browser: browser },
        page: page.into(),
    })
}

fn runtime() -> &'static Runtime {
    static RUNTIME: std::sync::OnceLock<Runtime> = std::sync::OnceLock::new();
    RUNTIME.get_or_init(|| Runtime::new().expect("failed to build a Tokio runtime"))
}

fn chrome_from_env(headless: bool) -> Chrome {
    let mut chrome = Chrome::installed().headless(headless);
    if let Ok(profile) = std::env::var("RUSTWRIGHT_PROFILE") {
        chrome = chrome.profile(profile);
    }
    chrome
}

fn firefox_from_env(headless: bool) -> Firefox {
    let mut firefox = Firefox::installed().headless(headless);
    if let Ok(profile) = std::env::var("RUSTWRIGHT_PROFILE") {
        firefox = firefox.profile(profile);
    }
    firefox
}
