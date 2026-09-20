use std::io;

use thiserror::Error;

/// Errors produced by the CDP transport and protocol layer.
#[derive(Debug, Error)]
pub enum CdpError {
    /// The WebSocket transport failed.
    #[error("websocket error: {0}")]
    WebSocket(#[from] tokio_tungstenite::tungstenite::Error),

    /// The browser returned a protocol-level error for a command.
    #[error("CDP command `{method}` failed ({code}): {message}")]
    Protocol {
        /// The command that failed, for example `Page.navigate`.
        method: String,
        /// Numeric CDP error code.
        code: i64,
        /// Human-readable CDP error message.
        message: String,
        /// Optional structured error data returned by the browser.
        data: Option<String>,
    },

    /// The connection was closed before a response arrived.
    #[error("CDP connection closed")]
    Closed,

    /// A message could not be (de)serialized as JSON.
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    /// The DevTools HTTP endpoint returned an unexpected response.
    #[error("devtools http error: {0}")]
    Http(String),

    /// An I/O error occurred while talking to the DevTools HTTP endpoint.
    #[error("io error: {0}")]
    Io(#[from] io::Error),

    /// A command did not receive a response before its deadline.
    #[error("CDP command `{method}` timed out after {timeout:?}")]
    Timeout {
        /// The command that timed out.
        method: String,
        /// The configured timeout.
        timeout: std::time::Duration,
    },
}

/// Convenience result alias for the CDP crate.
pub type CdpResult<T> = Result<T, CdpError>;
