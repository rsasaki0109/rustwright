use std::time::Duration;

use thiserror::Error;

/// Errors produced by the Rustwright object model.
#[derive(Debug, Error)]
pub enum Error {
    /// The browser could not be discovered, launched or driven.
    #[error(transparent)]
    Browser(#[from] rustwright_browser::BrowserError),

    /// A CDP transport or protocol error.
    #[error(transparent)]
    Cdp(#[from] rustwright_cdp::CdpError),

    /// An element matching the locator could not be found.
    #[error("no element found for {selector}")]
    ElementNotFound {
        /// A human-readable description of the selector.
        selector: String,
    },

    /// An operation exceeded its deadline.
    #[error("timed out after {timeout:?} waiting for {what}")]
    Timeout {
        /// What was being awaited.
        what: String,
        /// The configured deadline.
        timeout: Duration,
    },

    /// JavaScript evaluation threw an exception.
    #[error("javascript error: {0}")]
    JavaScript(String),

    /// Navigation failed.
    #[error("navigation failed: {0}")]
    Navigation(String),

    /// The page has been closed.
    #[error("page has been closed")]
    PageClosed,

    /// The browser context has been closed.
    #[error("browser context has been closed")]
    ContextClosed,

    /// The browser connection has closed.
    #[error("browser connection has closed")]
    BrowserClosed,

    /// A JSON value could not be (de)serialized.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// An I/O error occurred.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

impl Error {
    /// Whether this error came from a closed browser or page.
    pub fn is_closed(&self) -> bool {
        matches!(
            self,
            Error::PageClosed | Error::ContextClosed | Error::BrowserClosed
        ) || matches!(self, Error::Cdp(rustwright_cdp::CdpError::Closed))
    }

    /// Whether this error represents an exceeded deadline.
    pub fn is_timeout(&self) -> bool {
        matches!(self, Error::Timeout { .. })
    }
}

/// Convenience result alias for Rustwright.
pub type Result<T> = std::result::Result<T, Error>;
