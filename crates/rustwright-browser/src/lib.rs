//! Browser discovery, launch, profiles and process lifecycle.
//!
//! Rustwright is *real-browser-first*: it drives the Chrome or Chromium that is
//! already installed on the machine instead of shipping a patched build. This
//! crate finds that browser, explains how to start it with remote debugging
//! enabled, and owns the resulting process.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod chrome;
mod discovery;
mod error;
mod firefox;
mod launch;

pub use chrome::{Chrome, ChromeVariant, Profile};
pub use discovery::{firefox_candidates, installed_candidates, path_candidates};
pub use error::{BrowserError, BrowserResult};
pub use firefox::{Firefox, LaunchedFirefox};
pub use launch::{LaunchedBrowser, DEFAULT_STARTUP_TIMEOUT};
