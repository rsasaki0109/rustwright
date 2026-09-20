//! Chrome DevTools Protocol (CDP) transport and typed protocol bindings.
//!
//! This crate is the low-level foundation of Rustwright. It is deliberately
//! independent of the higher-level object model: it only knows how to
//!
//! 1. discover a browser's WebSocket endpoint over the DevTools HTTP API,
//! 2. speak CDP over a single WebSocket using *flat* session multiplexing, and
//! 3. hand typed protocol messages to callers.
//!
//! # Design
//!
//! A [`CdpConnection`] owns one WebSocket to the browser-level endpoint. It
//! spawns a reader task that routes responses to the matching request by `id`
//! and broadcasts events to subscribers. [`CdpSession`] is a thin handle that
//! pins a `sessionId` and forwards calls through the same connection.
//!
//! The transport is intentionally small and uses no `unsafe` code.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod connection;
mod error;
mod http;
pub mod protocol;
mod session;

pub use connection::{CdpConnection, CdpEvent, DISCONNECTED_EVENT};
pub use error::{CdpError, CdpResult};
pub use http::{discover_ws_url, http_get, http_get_json, HttpEndpoint};
pub use protocol::version::BrowserVersion;
pub use session::CdpSession;
