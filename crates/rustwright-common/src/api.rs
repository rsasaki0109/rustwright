//! A backend-agnostic page and locator API.
//!
//! The CDP backend (`rustwright-core`) and the BiDi backend
//! (`rustwright-bidi`) implement these traits, so application or test code can
//! be written once and run against either browser.
//!
//! The traits are generic over an associated [`PageApi::Error`], which lets each
//! backend keep its own precise error type (and causal chain) without a lossy
//! conversion layer. Dispatch is static: use them as bounds
//! (`fn f<P: PageApi>(page: &P)`) rather than as `dyn` objects.
//!
//! ```no_run
//! use rustwright_common::{LocatorApi, PageApi, Selector};
//!
//! async fn search<P>(page: &P, query: &str) -> Result<String, P::Error>
//! where
//!     P: PageApi,
//!     P::Locator: LocatorApi<Error = P::Error>,
//! {
//!     page.goto("https://example.com").await?;
//!     page.locator(Selector::css("input[name=q]")).fill(query).await?;
//!     page.locator(Selector::css("h1")).text().await
//! }
//! ```

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::{Role, Selector, WaitState};

/// A page driven by some backend.
///
/// All methods are async and return the backend's error type. Use the
/// `get_by_*` helpers to build semantic locators.
#[allow(async_fn_in_trait)]
pub trait PageApi {
    /// The backend's error type.
    type Error: std::error::Error + Send + Sync + 'static;

    /// The backend's locator type.
    type Locator: LocatorApi<Error = Self::Error>;

    /// Navigate to `url` and wait for the page to load.
    async fn goto(&self, url: &str) -> Result<(), Self::Error>;

    /// The current URL.
    async fn url(&self) -> Result<String, Self::Error>;

    /// The document title.
    async fn title(&self) -> Result<String, Self::Error>;

    /// The serialized HTML of the page, including the doctype.
    async fn content(&self) -> Result<String, Self::Error>;

    /// Evaluate a JavaScript expression and return its value.
    async fn evaluate(&self, expression: &str) -> Result<Value, Self::Error>;

    /// Take a screenshot and write it to `path`.
    async fn screenshot(&self, path: &Path) -> Result<(), Self::Error>;

    /// Override the viewport.
    async fn set_viewport(
        &self,
        width: i64,
        height: i64,
        device_pixel_ratio: f64,
    ) -> Result<(), Self::Error>;

    /// Close the page.
    async fn close(&self) -> Result<(), Self::Error>;

    /// Create a locator for a selector strategy.
    fn locator(&self, selector: Selector) -> Self::Locator;

    /// Create a locator that matches elements by their text.
    fn get_by_text(&self, text: &str) -> Self::Locator {
        self.locator(Selector::text(text, false))
    }

    /// Create a locator that matches elements by their exact text.
    fn get_by_text_exact(&self, text: &str) -> Self::Locator {
        self.locator(Selector::text(text, true))
    }

    /// Create a semantic locator by ARIA role and optional accessible name.
    fn get_by_role(&self, role: Role, name: Option<&str>) -> Self::Locator {
        self.locator(Selector::role(role, name))
    }

    /// Create a locator by `placeholder` attribute.
    fn get_by_placeholder(&self, text: &str) -> Self::Locator {
        self.locator(Selector::placeholder(text, false))
    }

    /// Create a locator by associated `<label>`.
    fn get_by_label(&self, text: &str) -> Self::Locator {
        self.locator(Selector::label(text, false))
    }

    /// Create a locator by image `alt` text.
    fn get_by_alt_text(&self, text: &str) -> Self::Locator {
        self.locator(Selector::alt_text(text, false))
    }

    /// Create a locator by test id.
    fn get_by_test_id(&self, id: &str) -> Self::Locator {
        self.locator(Selector::test_id(id))
    }
}

/// A lazy, auto-waiting element locator.
#[allow(async_fn_in_trait)]
pub trait LocatorApi {
    /// The backend's error type.
    type Error: std::error::Error + Send + Sync + 'static;

    /// Click the element.
    async fn click(&self) -> Result<(), Self::Error>;

    /// Replace the element's value.
    async fn fill(&self, text: &str) -> Result<(), Self::Error>;

    /// The element's rendered (inner) text, trimmed.
    async fn text(&self) -> Result<String, Self::Error>;

    /// The element's `textContent`, trimmed.
    async fn text_content(&self) -> Result<String, Self::Error>;

    /// Whether the element exists and is visible.
    async fn is_visible(&self) -> Result<bool, Self::Error>;

    /// Whether the element is absent or not visible.
    async fn is_hidden(&self) -> Result<bool, Self::Error>;

    /// Whether the element is enabled.
    async fn is_enabled(&self) -> Result<bool, Self::Error>;

    /// The number of elements matching the selector.
    async fn count(&self) -> Result<usize, Self::Error>;

    /// The value of an attribute, or `None` if absent.
    async fn get_attribute(&self, name: &str) -> Result<Option<String>, Self::Error>;

    /// Move the mouse over the element.
    async fn hover(&self) -> Result<(), Self::Error>;

    /// Scroll the element into the viewport.
    async fn scroll_into_view_if_needed(&self) -> Result<(), Self::Error>;

    /// Wait for the element to reach `state`, using the backend default timeout.
    async fn wait_for(&self, state: WaitState) -> Result<(), Self::Error>;

    /// Wait for the element to reach `state` with an explicit timeout.
    async fn wait_for_with_timeout(
        &self,
        state: WaitState,
        timeout: Duration,
    ) -> Result<(), Self::Error>;

    /// The first matching element.
    fn first(&self) -> Self
    where
        Self: Sized;

    /// The last matching element.
    fn last(&self) -> Self
    where
        Self: Sized;

    /// The `n`-th matching element. Negative indices count from the end.
    fn nth(&self, index: i64) -> Self
    where
        Self: Sized;
}
