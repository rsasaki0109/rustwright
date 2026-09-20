//! Core object model for Rustwright: browsers, contexts, pages and locators.
//!
//! Most users should depend on the `rustwright` facade crate, which re-exports
//! this crate together with a [`prelude`].
//!
//! The hierarchy mirrors Playwright:
//!
//! ```text
//! Browser -> BrowserContext -> Page -> Locator
//! ```
//!
//! Everything is async and built on Tokio. Actions auto-wait, so code rarely
//! needs explicit sleeps.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod api;
mod browser;
mod context;
mod diagnostics;
mod error;
mod frame;
mod locator;
mod page;

pub use browser::Browser;
pub use context::BrowserContext;
pub use diagnostics::{
    BrowserDiagnostics, ConsoleMessage, DialogInfo, NavigationEvent, NetworkRequest, OriginStorage,
    PageError, StorageItem, StorageState,
};
pub use error::{Error, Result};
pub use frame::{Frame, FrameLocator};
pub use locator::Locator;
pub use page::{normalize_url, Page, TracingOptions, Viewport, DEFAULT_TIMEOUT};
pub use rustwright_common::{
    glob_match, LoadState, LocatorApi, PageApi, Role, Route, RouteAction, Selector, WaitState,
};

pub use rustwright_browser::{
    BrowserError, Chrome, ChromeVariant, Firefox, LaunchedBrowser, LaunchedFirefox, Profile,
};
pub use rustwright_cdp::{BrowserVersion, CdpError};

/// The types most programs need.
///
/// ```
/// use rustwright_core::prelude::*;
/// ```
pub mod prelude {
    pub use crate::{
        Browser, BrowserContext, Chrome, Error, Firefox, LoadState, Locator, LocatorApi, Page,
        PageApi, Result, Role, Selector, WaitState,
    };
}
