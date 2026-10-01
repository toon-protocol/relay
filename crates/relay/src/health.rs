//! `GET /health`: liveness plus the node's identity and version. It is what
//! the image's `HEALTHCHECK` and every stack's dependency check probe, so its
//! body shape is frozen (#185, story 43).

use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::extract::State;
use nostr::key::PublicKey;
use serde::Serialize;

/// The body: the TypeScript relay's keys, in its order.
#[derive(Serialize)]
pub(crate) struct Health {
    status: &'static str,
    pubkey: String,
    capabilities: [&'static str; 1],
    version: &'static str,
    /// Milliseconds since the Unix epoch.
    timestamp: u64,
}

pub(crate) async fn health(State(identity): State<PublicKey>) -> Json<Health> {
    Json(Health {
        status: "healthy",
        pubkey: identity.to_hex(),
        capabilities: ["relay"],
        version: env!("CARGO_PKG_VERSION"),
        timestamp: unix_millis(),
    })
}

/// A clock set before 1970 reads as 0 rather than failing a liveness probe.
fn unix_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| {
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
        })
}
