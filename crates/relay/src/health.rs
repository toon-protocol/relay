//! `GET /health`: liveness plus the node's identity and version. It is what
//! the image's `HEALTHCHECK` and every stack's dependency check probe, so its
//! body shape is frozen (#185, story 43).

use axum::Json;
use axum::extract::State;
use serde::Serialize;

use crate::Relay;
use crate::clock::unix_millis;

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

pub(crate) async fn health(State(relay): State<Relay>) -> Json<Health> {
    Json(Health {
        status: "healthy",
        pubkey: relay.identity.to_hex(),
        capabilities: ["relay"],
        version: env!("CARGO_PKG_VERSION"),
        timestamp: unix_millis(),
    })
}
