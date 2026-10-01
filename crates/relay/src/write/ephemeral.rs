//! `POST /write-ephemeral`: the free lane for ephemeral events (kinds 20000
//! to 29999). The connector terminates it on its own zero-priced route, so
//! nothing here was paid for, and the lane is bounded instead:
//!
//! 1. The rate limit runs first, before the body is read, so a caller over
//!    budget costs as little as possible: `429`.
//! 2. The body cap runs before the body is parsed: `413`.
//! 3. A body that is not JSON, or has no event: `400`.
//! 4. A kind outside the ephemeral range: `400`. A persistent kind on a free
//!    lane would be a free ride around pay-to-write.
//! 5. The signature is always verified, with no way to skip it: it is the
//!    only defence against forged events being broadcast for free. `422`.
//! 6. The event is delivered to live subscribers. It is never stored.

use std::net::SocketAddr;
use std::time::Duration;

use axum::Json;
use axum::body::{Body, to_bytes};
use axum::extract::{ConnectInfo, Request, State};
use axum::http::StatusCode;
use axum::http::header::CONTENT_LENGTH;
use axum::response::{IntoResponse, Response};
use http_body_util::LengthLimitError;
use serde::Serialize;
use serde_json::Value;

use super::limiter::RateLimiter;
use super::{event_value, kind_of, log_line, refused, verified_event};
use crate::clock::unix_seconds;
use crate::{Config, Relay};

/// The lane's bounds, from the configuration.
#[derive(Debug)]
pub(crate) struct Lane {
    limiter: RateLimiter,
    max_body_bytes: usize,
}

impl Lane {
    pub(crate) fn new(config: &Config) -> Self {
        Self {
            limiter: RateLimiter::new(
                config.ephemeral_rate_limit,
                Duration::from_millis(config.ephemeral_rate_window_ms),
            ),
            max_body_bytes: usize::try_from(config.ephemeral_max_body_bytes).unwrap_or(usize::MAX),
        }
    }
}

/// Whether `kind` is ephemeral (NIP-16).
fn is_ephemeral(kind: u64) -> bool {
    (20_000..30_000).contains(&kind)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Broadcast {
    event_id: String,
    /// Seconds since the Unix epoch.
    broadcast_at: u64,
}

pub(crate) async fn write_ephemeral(State(relay): State<Relay>, request: Request) -> Response {
    // Behind the connector every caller is the connector, so in the canonical
    // deploy this is a lane-wide cap. A server with no connection info shares
    // one bucket, which fails towards a cap rather than none.
    let client = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or_else(|| "unknown".to_string(), |info| info.0.ip().to_string());
    if !relay.ephemeral.limiter.allow(&client) {
        return refused(StatusCode::TOO_MANY_REQUESTS, "Rate limit exceeded");
    }

    let cap = relay.ephemeral.max_body_bytes;
    let too_large = || refused(StatusCode::PAYLOAD_TOO_LARGE, "Request body too large");
    let declared = request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());
    if declared.is_some_and(|length| length > cap as u64) {
        return too_large();
    }
    let body: Body = request.into_body();
    let bytes = match to_bytes(body, cap).await {
        Ok(bytes) => bytes,
        Err(error) => {
            let over_cap = std::error::Error::source(&error)
                .is_some_and(|source| source.is::<LengthLimitError>());
            return if over_cap {
                too_large()
            } else {
                refused(StatusCode::BAD_REQUEST, "Invalid request body")
            };
        }
    };

    let Ok(body) = serde_json::from_slice::<Value>(&bytes) else {
        return refused(StatusCode::BAD_REQUEST, "Invalid request body");
    };
    let event = match event_value(&body) {
        Ok(event) => event,
        Err(refusal) => return refusal.into_response(),
    };
    if !kind_of(&event).is_some_and(is_ephemeral) {
        return refused(
            StatusCode::BAD_REQUEST,
            "Only ephemeral kinds (20000-29999) are accepted on this lane",
        );
    }
    if relay.log_writes {
        println!("{}", log_line(&event, "write-ephemeral", None));
    }
    let event = match verified_event(event) {
        Ok(event) => event,
        Err(refusal) => return refusal.into_response(),
    };

    relay.read_side.deliver(&event);
    Json(Broadcast {
        event_id: event.event().id.to_hex(),
        broadcast_at: unix_seconds(),
    })
    .into_response()
}
