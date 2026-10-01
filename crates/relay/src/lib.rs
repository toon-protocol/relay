//! The TOON relay, rebuilt in Rust as a drop-in replacement for the
//! TypeScript image (#185).
//!
//! This is the first slice (#192): the path from source to a conformance-gated
//! image, carrying the thinnest behaviour that proves it. The relay reads its
//! identity and its write listener from the environment and answers
//! `GET /health`. Every other surface in #185's compatibility contract is a
//! later slice, and until it lands the conformance suite lists it as an
//! expected failure for this implementation.
//!
//! It is a library only so that the router can be driven in a test without a
//! socket. Nothing is published and nothing outside this workspace imports it.

mod config;
mod error;
mod health;

pub use config::Config;
pub use error::RelayError;

use axum::Router;
use axum::routing::get;

/// Everything served on the write port.
///
/// The caller binds: a router that does not own its port can be driven
/// in a test with no listener.
pub fn write_router(config: &Config) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .with_state(config.identity)
}
