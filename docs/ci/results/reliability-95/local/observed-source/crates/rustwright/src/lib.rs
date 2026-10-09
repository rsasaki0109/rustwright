//! # Rustwright
//!
//! Real-browser-first browser automation for Rust.
//!
//! Rustwright drives the Chrome or Chromium already installed on your machine
//! over the Chrome DevTools Protocol, exposing a Playwright-like async API:
//!
//! ```no_run
//! use rustwright::prelude::*;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     let browser = Browser::launch(Chrome::installed().headless(false)).await?;
//!     let page = browser.new_page().await?;
//!     page.goto("example.com").await?;
//!     println!("{}", page.title().await?);
//!     page.screenshot("page.png").await?;
//!     browser.close().await?;
//!     Ok(())
//! }
//! ```
//!
//! ## Philosophy
//!
//! Rustwright is *not* an anti-bot bypass framework. It does not solve CAPTCHAs,
//! spoof fingerprints, hide `navigator.webdriver`, rotate proxies or evade rate
//! limits. Instead it minimizes the delta between automated and normal browsing
//! by driving a standard, unmodified browser with a persistent or ephemeral
//! profile, and by making any remaining difference observable through
//! diagnostics.
//!
//! ## Structure
//!
//! - [`Browser`] / [`BrowserContext`] / [`Page`] / [`Locator`] — the object model.
//! - [`Chrome`] — Chrome/Chromium discovery, launch and profiles.
//! - [`browser`] / [`cdp`] — lower-level crates for advanced use.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod any;

pub use any::{AnyError, AnyLocator, AnyPage};
pub use rustwright_core::*;

/// WebDriver BiDi client and Firefox driver.
pub use rustwright_bidi as bidi;
/// Lower-level browser process management.
pub use rustwright_browser as browser;
/// Low-level Chrome DevTools Protocol bindings.
pub use rustwright_cdp as cdp;

/// Types for driving Firefox over WebDriver BiDi.
pub use rustwright_bidi::{
    BidiBrowser, BidiConnection, BidiError, BidiEvent, BidiLocator, BidiPage, BidiResult,
    BidiSession, BrowsingContextInfo,
};

/// The types most programs need, including [`Browser`], [`Chrome`], [`Page`],
/// [`Locator`] and the Rustwright [`Result`].
pub mod prelude {
    pub use crate::{AnyError, AnyLocator, AnyPage};
    pub use rustwright_bidi::{BidiBrowser, BidiPage, BidiSession};
    pub use rustwright_core::prelude::*;
    pub use rustwright_core::{
        glob_match, BrowserDiagnostics, BrowserError, BrowserVersion, CdpError, ChromeVariant,
        ConsoleMessage, DialogInfo, Frame, FrameLocator, NavigationEvent, NetworkRequest,
        OriginStorage, PageError, Profile, Route, RouteAction, StorageItem, StorageState,
        TracingOptions, Viewport,
    };
}
