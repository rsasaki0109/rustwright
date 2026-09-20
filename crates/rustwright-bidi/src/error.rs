use std::io;

use thiserror::Error;

/// Errors produced by the WebDriver BiDi client.
#[derive(Debug, Error)]
pub enum BidiError {
    /// The browser process could not be launched.
    #[error(transparent)]
    Browser(#[from] rustwright_browser::BrowserError),

    /// The WebSocket transport failed.
    #[error("bidi websocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    /// The remote end returned a BiDi error.
    #[error("bidi command `{method}` failed: {error}: {message}")]
    Protocol {
        /// The command that failed.
        method: String,
        /// The BiDi error code.
        error: String,
        /// The human-readable message.
        message: String,
    },

    /// The connection closed before a response arrived.
    #[error("bidi connection closed")]
    Closed,

    /// A message could not be (de)serialized.
    #[error("bidi json error: {0}")]
    Json(#[from] serde_json::Error),

    /// An I/O error occurred.
    #[error("bidi io error: {0}")]
    Io(#[from] io::Error),

    /// A command did not receive a response in time.
    #[error("bidi command `{method}` timed out after {timeout:?}")]
    Timeout {
        /// The command that timed out.
        method: String,
        /// The configured timeout.
        timeout: std::time::Duration,
    },

    /// A response was missing an expected field.
    #[error("unexpected bidi response: {0}")]
    Unexpected(String),

    /// No element matched a locator.
    #[error("no element found for {0}")]
    ElementNotFound(String),

    /// A wait exceeded its deadline.
    #[error("timed out waiting for {0}")]
    WaitTimeout(String),
}

/// Convenience result alias for the BiDi crate.
pub type BidiResult<T> = Result<T, BidiError>;
