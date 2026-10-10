//! Typed bindings for the subset of CDP used by Rustwright.
//!
//! Only the domains Rustwright actually speaks are modelled; the vocabulary is
//! intentionally small rather than fully code-generated, which keeps compile
//! times and the public surface manageable. Every type round-trips as the exact
//! JSON shape Chrome expects.

pub mod browser;
pub mod dom;
pub mod emulation;
pub mod fetch;
pub mod input;
pub mod network;
pub mod page;
pub mod runtime;
pub mod target;
pub mod tracing;
pub mod version;
