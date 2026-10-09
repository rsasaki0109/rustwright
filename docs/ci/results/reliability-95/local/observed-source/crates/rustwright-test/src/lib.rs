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
//! WebDriver BiDi. Tests skip (with a message) when automatic discovery finds no
//! browser. Invalid explicit paths and startup or connection failures fail the
//! test and use the normal retry policy.
//!
//! Environment variables:
//!
//! - `RUSTWRIGHT_BROWSER=firefox` runs against Firefox/BiDi (default: Chrome).
//! - `RUSTWRIGHT_HEADLESS=0` runs a visible browser (default: headless).
//! - `RUSTWRIGHT_RETRIES=N` retries a failing test up to `N` extra times.
//! - `RUSTWRIGHT_SHARD=i/N` runs only this shard's tests (1-based index).
//! - `RUSTWRIGHT_PROFILE=/path` uses a persistent profile.
//! - `RUSTWRIGHT_CHROME` / `RUSTWRIGHT_FIREFOX` pin the executable.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod expect;

use std::future::Future;

use tokio::runtime::Runtime;

pub use expect::{expect, Expectation};
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
    pub use crate::{expect, rustwright_test, LocatorApi, PageApi, Result, TestContext};
    pub use crate::{Role, Selector, WaitState};
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
    F: Fn(TestContext) -> Fut + Send + Sync + 'static,
    Fut: Future<Output = Result<()>> + Send + 'static,
{
    if let Some((index, total)) = shard() {
        if !in_shard(name, index, total) {
            eprintln!("skipping test `{name}` (shard {index}/{total})");
            return;
        }
    }

    let attempts = retries() + 1;
    let mut last_error = String::new();

    for attempt in 1..=attempts {
        // Catch panics too, so assertion failures can be retried.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            runtime().block_on(run_attempt(&test))
        }))
        .unwrap_or_else(|payload| Outcome::Failed(panic_message(&payload)));

        match outcome {
            Outcome::Passed => return,
            Outcome::Skipped(reason) => {
                eprintln!("skipping test `{name}`: {reason}");
                return;
            }
            Outcome::Failed(message) => last_error = message,
        }

        if attempt < attempts {
            eprintln!(
                "test `{name}` failed (attempt {attempt}/{attempts}), retrying: {last_error}"
            );
        }
    }

    panic!("test `{name}` failed after {attempts} attempt(s): {last_error}");
}

enum Outcome {
    Passed,
    Skipped(String),
    Failed(String),
}

async fn run_attempt<F, Fut>(test: &F) -> Outcome
where
    F: Fn(TestContext) -> Fut,
    Fut: Future<Output = Result<()>>,
{
    let headless = std::env::var("RUSTWRIGHT_HEADLESS")
        .map(|value| value != "0")
        .unwrap_or(true);
    let backend = std::env::var("RUSTWRIGHT_BROWSER").unwrap_or_default();

    let (browser_name, launched) = match backend.as_str() {
        "firefox" | "bidi" => ("firefox", launch_firefox(headless).await),
        _ => ("chrome", launch_chrome(headless).await),
    };
    let context = match launched {
        Ok(context) => context,
        Err(error) => return setup_failure(browser_name, error),
    };

    match test(context).await {
        Ok(()) => Outcome::Passed,
        Err(error) => Outcome::Failed(error.to_string()),
    }
}

fn setup_failure(browser_name: &str, error: AnyError) -> Outcome {
    let not_installed = matches!(
        &error,
        AnyError::Chrome(rustwright::Error::Browser(
            rustwright::BrowserError::NotFound { .. }
        )) | AnyError::Firefox(rustwright::BidiError::Browser(
            rustwright::BrowserError::NotFound { .. }
        ))
    );
    if not_installed {
        Outcome::Skipped(format!("{browser_name} unavailable: {error}"))
    } else {
        Outcome::Failed(format!("{browser_name} setup failed: {error}"))
    }
}

fn retries() -> u32 {
    std::env::var("RUSTWRIGHT_RETRIES")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(0)
}

/// Parse `RUSTWRIGHT_SHARD` as `index/total` (1-based).
fn shard() -> Option<(u32, u32)> {
    let value = std::env::var("RUSTWRIGHT_SHARD").ok()?;
    let (index, total) = value.split_once('/')?;
    let index: u32 = index.trim().parse().ok()?;
    let total: u32 = total.trim().parse().ok()?;
    if index == 0 || total == 0 || index > total {
        return None;
    }
    Some((index, total))
}

/// Deterministic shard assignment by test name.
fn in_shard(name: &str, index: u32, total: u32) -> bool {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    name.hash(&mut hasher);
    (hasher.finish() % u64::from(total)) as u32 + 1 == index
}

fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_string()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "test panicked".to_string()
    }
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
    let mut chrome = match std::env::var_os("RUSTWRIGHT_CHROME") {
        Some(path) => Chrome::at(path),
        None => Chrome::installed(),
    }
    .headless(headless);
    if let Ok(profile) = std::env::var("RUSTWRIGHT_PROFILE") {
        chrome = chrome.profile(profile);
    }
    chrome
}

fn firefox_from_env(headless: bool) -> Firefox {
    let mut firefox = match std::env::var_os("RUSTWRIGHT_FIREFOX") {
        Some(path) => Firefox::at(path),
        None => Firefox::installed(),
    }
    .headless(headless);
    if let Ok(profile) = std::env::var("RUSTWRIGHT_PROFILE") {
        firefox = firefox.profile(profile);
    }
    firefox
}

#[cfg(test)]
mod tests {
    use super::{in_shard, setup_failure, AnyError, Outcome};
    use rustwright::{BidiError, BrowserError, CdpError, Error};

    fn wrap_browser_error(browser: &str, error: BrowserError) -> AnyError {
        match browser {
            "chrome" => Error::Browser(error).into(),
            "firefox" => BidiError::Browser(error).into(),
            _ => unreachable!(),
        }
    }

    #[test]
    fn automatic_discovery_absence_skips_both_backends() {
        for browser in ["chrome", "firefox"] {
            let error = wrap_browser_error(
                browser,
                BrowserError::NotFound {
                    search_paths: vec!["missing-browser".into()],
                },
            );
            assert!(
                matches!(setup_failure(browser, error), Outcome::Skipped(message)
                if message.starts_with(&format!("{browser} unavailable:")))
            );
        }
    }

    #[test]
    fn other_browser_setup_errors_fail_both_backends() {
        for browser in ["chrome", "firefox"] {
            let errors = [
                BrowserError::ExecutableNotFound("missing-browser".into()),
                BrowserError::Launch {
                    executable: "browser".into(),
                    source: std::io::ErrorKind::PermissionDenied.into(),
                },
                BrowserError::StartupTimeout {
                    timeout: std::time::Duration::from_secs(1),
                    log_path: "browser.log".into(),
                },
                BrowserError::EarlyExit {
                    status: "exit status: 1".into(),
                    log_path: "browser.log".into(),
                },
                BrowserError::ProfileDir {
                    path: "profile".into(),
                    source: std::io::ErrorKind::PermissionDenied.into(),
                },
                BrowserError::InvalidActivePort {
                    path: "DevToolsActivePort".into(),
                    contents: "invalid".into(),
                },
                BrowserError::Io(std::io::ErrorKind::ConnectionRefused.into()),
                BrowserError::Cdp(CdpError::Closed),
            ];
            for error in errors {
                let expected = error.to_string();
                let outcome = setup_failure(browser, wrap_browser_error(browser, error));
                assert!(matches!(outcome, Outcome::Failed(message)
                    if message.starts_with(&format!("{browser} setup failed:")) && message.contains(&expected)));
            }
        }
    }

    #[test]
    fn connection_and_context_setup_errors_fail() {
        for (browser, error) in [
            ("chrome", AnyError::Chrome(Error::Cdp(CdpError::Closed))),
            ("chrome", AnyError::Chrome(Error::ContextClosed)),
            (
                "firefox",
                AnyError::Firefox(BidiError::Unexpected("session creation failed".into())),
            ),
        ] {
            let expected = error.to_string();
            assert!(
                matches!(setup_failure(browser, error), Outcome::Failed(message)
                if message.contains(&expected))
            );
        }
    }

    #[test]
    fn shards_partition_tests() {
        let total = 3;
        for index in 0..40 {
            let name = format!("test_{index}");
            let matches = (1..=total)
                .filter(|shard| in_shard(&name, *shard, total))
                .count();
            assert_eq!(matches, 1, "each test belongs to exactly one shard");
        }
    }
}
