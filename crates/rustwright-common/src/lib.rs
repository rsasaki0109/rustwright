//! Backend-agnostic types shared by Rustwright's CDP and BiDi backends.
//!
//! Selectors, wait states and the page-side helper script are independent of
//! any wire protocol, so they live here and are reused by `rustwright-core`
//! (Chrome/CDP) and `rustwright-bidi` (Firefox/BiDi). This is also the
//! foundation for a future unified `Page` abstraction.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod api;
mod selectors;
mod wait;

pub use api::{LocatorApi, PageApi};
pub use selectors::{Role, Selector};
pub use wait::{LoadState, WaitState};

/// The helper script installed into every document.
///
/// It defines `window.__rustwright` with `resolve`, `queryAll`, `isVisible`,
/// `isEnabled`, `count` and `waitFor`, all driven by the JSON specs produced by
/// [`Selector::to_spec`].
pub const INJECTED_SCRIPT: &str = include_str!("injected.js");
