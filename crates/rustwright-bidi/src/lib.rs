//! WebDriver BiDi client and Firefox driver for Rustwright.
//!
//! Rustwright's primary transport is CDP (see `rustwright-cdp`), but the
//! roadmap calls for WebDriver BiDi and Firefox support. This crate provides
//! that separate transport: a BiDi WebSocket client plus a minimal Firefox
//! launcher and a `BidiBrowser` / `BidiPage` built on top.
//!
//! ```no_run
//! use rustwright_bidi::{BidiBrowser, BidiResult};
//! use rustwright_browser::Firefox;
//!
//! # async fn run() -> BidiResult<()> {
//! let browser = BidiBrowser::launch(Firefox::installed().headless(true)).await?;
//! let page = browser.new_page().await?;
//! page.goto("example.com").await?;
//! println!("{}", page.title().await?);
//! browser.close().await?;
//! # Ok(())
//! # }
//! ```

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod api;
mod browser;
mod connection;
mod error;
mod frame;
mod locator;
mod network;
mod session;

pub use browser::{BidiBrowser, BidiContext, BidiPage};
pub use connection::{BidiConnection, BidiEvent};
pub use error::{BidiError, BidiResult};
pub use frame::BidiFrame;
pub use locator::BidiLocator;
pub use network::BidiNetworkRequest;
pub use session::{
    BidiCookie, BidiOriginStorage, BidiSession, BidiStorageItem, BidiStorageState,
    BrowsingContextInfo,
};
