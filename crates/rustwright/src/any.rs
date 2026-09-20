//! Runtime-selected pages: one concrete type that can hold either backend.
//!
//! The [`PageApi`] / [`LocatorApi`] traits are generic over an associated error,
//! which means static dispatch. When a single type is needed — for example a
//! `Vec<AnyPage>` in a test harness that may run against Chrome or Firefox —
//! these enums provide dynamic dispatch without trait objects.
//!
//! ```
//! use rustwright::{AnyPage, PageApi};
//! use rustwright::prelude::Selector;
//!
//! # async fn run(pages: Vec<AnyPage>) -> Result<(), rustwright::AnyError> {
//! for page in &pages {
//!     page.goto("example.com").await?;
//!     println!("{}", page.title().await?);
//! }
//! # Ok(())
//! # }
//! ```

use std::path::Path;
use std::time::Duration;

use rustwright_bidi::{BidiError, BidiLocator as FirefoxLocator, BidiPage as FirefoxPage};
use rustwright_common::{LocatorApi, PageApi, Selector, WaitState};
use rustwright_core::{
    Error as ChromeError, Locator as ChromeLocator, Page as ChromePage, Viewport,
};
use serde_json::Value;
use thiserror::Error;

/// An error from either backend.
#[derive(Debug, Error)]
pub enum AnyError {
    /// A Chrome/CDP error.
    #[error(transparent)]
    Chrome(#[from] ChromeError),
    /// A Firefox/BiDi error.
    #[error(transparent)]
    Firefox(#[from] BidiError),
}

/// A page from either backend.
#[derive(Clone)]
pub enum AnyPage {
    /// A Chrome page driven over CDP.
    Chrome(ChromePage),
    /// A Firefox page driven over WebDriver BiDi.
    Firefox(FirefoxPage),
}

impl std::fmt::Debug for AnyPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnyPage::Chrome(page) => f.debug_tuple("AnyPage::Chrome").field(page).finish(),
            AnyPage::Firefox(page) => f.debug_tuple("AnyPage::Firefox").field(page).finish(),
        }
    }
}

impl From<ChromePage> for AnyPage {
    fn from(page: ChromePage) -> Self {
        AnyPage::Chrome(page)
    }
}

impl From<FirefoxPage> for AnyPage {
    fn from(page: FirefoxPage) -> Self {
        AnyPage::Firefox(page)
    }
}

impl AnyPage {
    /// Open a file or data URL in the appropriate browser, for diagnostics.
    pub fn backend_name(&self) -> &'static str {
        match self {
            AnyPage::Chrome(_) => "chrome",
            AnyPage::Firefox(_) => "firefox",
        }
    }
}

/// A locator from either backend.
#[derive(Clone)]
pub enum AnyLocator {
    /// A Chrome/CDP locator.
    Chrome(ChromeLocator),
    /// A Firefox/BiDi locator.
    Firefox(FirefoxLocator),
}

impl std::fmt::Debug for AnyLocator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AnyLocator::Chrome(locator) => {
                f.debug_tuple("AnyLocator::Chrome").field(locator).finish()
            }
            AnyLocator::Firefox(locator) => {
                f.debug_tuple("AnyLocator::Firefox").field(locator).finish()
            }
        }
    }
}

impl PageApi for AnyPage {
    type Error = AnyError;
    type Locator = AnyLocator;

    async fn goto(&self, url: &str) -> Result<(), Self::Error> {
        match self {
            AnyPage::Chrome(page) => ChromePage::goto(page, url).await.map_err(Into::into),
            AnyPage::Firefox(page) => FirefoxPage::goto(page, url).await.map_err(Into::into),
        }
    }

    async fn url(&self) -> Result<String, Self::Error> {
        match self {
            AnyPage::Chrome(page) => Ok(ChromePage::url(page)),
            AnyPage::Firefox(page) => FirefoxPage::url(page).await.map_err(Into::into),
        }
    }

    async fn title(&self) -> Result<String, Self::Error> {
        match self {
            AnyPage::Chrome(page) => ChromePage::title(page).await.map_err(Into::into),
            AnyPage::Firefox(page) => FirefoxPage::title(page).await.map_err(Into::into),
        }
    }

    async fn content(&self) -> Result<String, Self::Error> {
        match self {
            AnyPage::Chrome(page) => ChromePage::content(page).await.map_err(Into::into),
            AnyPage::Firefox(page) => FirefoxPage::content(page).await.map_err(Into::into),
        }
    }

    async fn evaluate(&self, expression: &str) -> Result<Value, Self::Error> {
        match self {
            AnyPage::Chrome(page) => ChromePage::evaluate(page, expression)
                .await
                .map_err(Into::into),
            AnyPage::Firefox(page) => FirefoxPage::evaluate(page, expression)
                .await
                .map_err(Into::into),
        }
    }

    async fn screenshot(&self, path: &Path) -> Result<(), Self::Error> {
        match self {
            AnyPage::Chrome(page) => ChromePage::screenshot(page, path).await.map_err(Into::into),
            AnyPage::Firefox(page) => FirefoxPage::screenshot(page, path)
                .await
                .map_err(Into::into),
        }
    }

    async fn set_viewport(
        &self,
        width: i64,
        height: i64,
        device_pixel_ratio: f64,
    ) -> Result<(), Self::Error> {
        match self {
            AnyPage::Chrome(page) => ChromePage::set_viewport(
                page,
                &Viewport {
                    width,
                    height,
                    device_scale_factor: device_pixel_ratio,
                    mobile: false,
                },
            )
            .await
            .map_err(Into::into),
            AnyPage::Firefox(page) => {
                FirefoxPage::set_viewport(page, width, height, device_pixel_ratio)
                    .await
                    .map_err(Into::into)
            }
        }
    }

    async fn close(&self) -> Result<(), Self::Error> {
        match self {
            AnyPage::Chrome(page) => ChromePage::close(page).await.map_err(Into::into),
            AnyPage::Firefox(page) => FirefoxPage::close(page).await.map_err(Into::into),
        }
    }

    fn locator(&self, selector: Selector) -> Self::Locator {
        match self {
            AnyPage::Chrome(page) => AnyLocator::Chrome(ChromePage::locator(page, selector)),
            AnyPage::Firefox(page) => AnyLocator::Firefox(FirefoxPage::locator(page, selector)),
        }
    }
}

impl LocatorApi for AnyLocator {
    type Error = AnyError;

    async fn click(&self) -> Result<(), Self::Error> {
        match self {
            AnyLocator::Chrome(locator) => ChromeLocator::click(locator).await.map_err(Into::into),
            AnyLocator::Firefox(locator) => {
                FirefoxLocator::click(locator).await.map_err(Into::into)
            }
        }
    }

    async fn fill(&self, text: &str) -> Result<(), Self::Error> {
        match self {
            AnyLocator::Chrome(locator) => {
                ChromeLocator::fill(locator, text).await.map_err(Into::into)
            }
            AnyLocator::Firefox(locator) => FirefoxLocator::fill(locator, text)
                .await
                .map_err(Into::into),
        }
    }

    async fn text(&self) -> Result<String, Self::Error> {
        match self {
            AnyLocator::Chrome(locator) => ChromeLocator::text(locator).await.map_err(Into::into),
            AnyLocator::Firefox(locator) => FirefoxLocator::text(locator).await.map_err(Into::into),
        }
    }

    async fn text_content(&self) -> Result<String, Self::Error> {
        match self {
            AnyLocator::Chrome(locator) => ChromeLocator::text_content(locator)
                .await
                .map_err(Into::into),
            AnyLocator::Firefox(locator) => FirefoxLocator::text_content(locator)
                .await
                .map_err(Into::into),
        }
    }

    async fn is_visible(&self) -> Result<bool, Self::Error> {
        match self {
            AnyLocator::Chrome(locator) => {
                ChromeLocator::is_visible(locator).await.map_err(Into::into)
            }
            AnyLocator::Firefox(locator) => FirefoxLocator::is_visible(locator)
                .await
                .map_err(Into::into),
        }
    }

    async fn is_hidden(&self) -> Result<bool, Self::Error> {
        match self {
            AnyLocator::Chrome(locator) => {
                ChromeLocator::is_hidden(locator).await.map_err(Into::into)
            }
            AnyLocator::Firefox(locator) => {
                FirefoxLocator::is_hidden(locator).await.map_err(Into::into)
            }
        }
    }

    async fn is_enabled(&self) -> Result<bool, Self::Error> {
        match self {
            AnyLocator::Chrome(locator) => {
                ChromeLocator::is_enabled(locator).await.map_err(Into::into)
            }
            AnyLocator::Firefox(locator) => FirefoxLocator::is_enabled(locator)
                .await
                .map_err(Into::into),
        }
    }

    async fn count(&self) -> Result<usize, Self::Error> {
        match self {
            AnyLocator::Chrome(locator) => ChromeLocator::count(locator).await.map_err(Into::into),
            AnyLocator::Firefox(locator) => {
                FirefoxLocator::count(locator).await.map_err(Into::into)
            }
        }
    }

    async fn get_attribute(&self, name: &str) -> Result<Option<String>, Self::Error> {
        match self {
            AnyLocator::Chrome(locator) => ChromeLocator::get_attribute(locator, name)
                .await
                .map_err(Into::into),
            AnyLocator::Firefox(locator) => FirefoxLocator::get_attribute(locator, name)
                .await
                .map_err(Into::into),
        }
    }

    async fn hover(&self) -> Result<(), Self::Error> {
        match self {
            AnyLocator::Chrome(locator) => ChromeLocator::hover(locator).await.map_err(Into::into),
            AnyLocator::Firefox(locator) => {
                FirefoxLocator::hover(locator).await.map_err(Into::into)
            }
        }
    }

    async fn scroll_into_view_if_needed(&self) -> Result<(), Self::Error> {
        match self {
            AnyLocator::Chrome(locator) => ChromeLocator::scroll_into_view_if_needed(locator)
                .await
                .map_err(Into::into),
            AnyLocator::Firefox(locator) => FirefoxLocator::scroll_into_view_if_needed(locator)
                .await
                .map_err(Into::into),
        }
    }

    async fn wait_for(&self, state: WaitState) -> Result<(), Self::Error> {
        match self {
            AnyLocator::Chrome(locator) => ChromeLocator::wait_for(locator, state)
                .await
                .map_err(Into::into),
            AnyLocator::Firefox(locator) => FirefoxLocator::wait_for(locator, state)
                .await
                .map_err(Into::into),
        }
    }

    async fn wait_for_with_timeout(
        &self,
        state: WaitState,
        timeout: Duration,
    ) -> Result<(), Self::Error> {
        match self {
            AnyLocator::Chrome(locator) => {
                ChromeLocator::wait_for_with_timeout(locator, state, timeout)
                    .await
                    .map_err(Into::into)
            }
            AnyLocator::Firefox(locator) => {
                FirefoxLocator::wait_for_with_timeout(locator, state, timeout)
                    .await
                    .map_err(Into::into)
            }
        }
    }

    fn first(&self) -> Self {
        match self {
            AnyLocator::Chrome(locator) => AnyLocator::Chrome(ChromeLocator::first(locator)),
            AnyLocator::Firefox(locator) => AnyLocator::Firefox(FirefoxLocator::first(locator)),
        }
    }

    fn last(&self) -> Self {
        match self {
            AnyLocator::Chrome(locator) => AnyLocator::Chrome(ChromeLocator::last(locator)),
            AnyLocator::Firefox(locator) => AnyLocator::Firefox(FirefoxLocator::last(locator)),
        }
    }

    fn nth(&self, index: i64) -> Self {
        match self {
            AnyLocator::Chrome(locator) => AnyLocator::Chrome(ChromeLocator::nth(locator, index)),
            AnyLocator::Firefox(locator) => {
                AnyLocator::Firefox(FirefoxLocator::nth(locator, index))
            }
        }
    }
}
