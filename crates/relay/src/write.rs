//! `POST /write`: where a paid write arrives. The connector settled the
//! payment before it delivered, so a request that reaches this handler is
//! already paid for; what is left is to verify the event, store it, and hand
//! it to the read side's open subscriptions.
//!
//! The statuses are the connector's contract (#185, stories 29 to 31): `200`
//! with the event id and the stored-at time, `400` for a body that is not a
//! delivery, `422` for an event that does not verify. Payment attribution
//! (#194) and the ephemeral lane (#198) are not built yet.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use nostr::event::Event;
use serde::Serialize;
use serde_json::Value;

use crate::{Relay, RelayError, Saved, VerifiedEvent};

/// The `200` body.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    event_id: String,
    /// Seconds since the Unix epoch.
    stored_at: u64,
}

#[derive(Serialize)]
struct Refused {
    error: String,
}

fn refused(status: StatusCode, error: impl Into<String>) -> Response {
    let error = error.into();
    (status, Json(Refused { error })).into_response()
}

/// The body is read as bytes, not through a JSON extractor: the TypeScript
/// relay parses whatever arrives, whatever its `Content-Type`.
pub(crate) async fn write(State(relay): State<Relay>, body: Bytes) -> Response {
    let Ok(body) = serde_json::from_slice::<Value>(&body) else {
        return refused(StatusCode::BAD_REQUEST, "Invalid request body");
    };
    let event = match body.get("event") {
        None | Some(Value::Null) => {
            return refused(StatusCode::BAD_REQUEST, "Missing required field: event");
        }
        Some(event) => event.clone(),
    };
    let event = match serde_json::from_value::<Event>(event) {
        Ok(event) => event,
        Err(error) => {
            return refused(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("Invalid event: {error}"),
            );
        }
    };
    let event = match VerifiedEvent::verify(event) {
        Ok(event) => event,
        Err(RelayError::EventIdMismatch) => {
            return refused(StatusCode::UNPROCESSABLE_ENTITY, "Invalid event id");
        }
        Err(_) => {
            return refused(StatusCode::UNPROCESSABLE_ENTITY, "Invalid event signature");
        }
    };

    match relay.store.save(&event).await {
        Ok(Saved::New) => relay.read_side.deliver(&event),
        // Already held, so already delivered: the connector retried.
        Ok(Saved::Duplicate) => {}
        Err(error @ RelayError::KindNotStoredYet { .. }) => {
            return refused(StatusCode::NOT_IMPLEMENTED, error.to_string());
        }
        Err(error) => {
            eprintln!("write: event {} was not stored: {error}", event.event().id);
            return refused(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The event could not be stored",
            );
        }
    }

    Json(Stored {
        event_id: event.event().id.to_hex(),
        stored_at: unix_seconds(),
    })
    .into_response()
}

/// A clock set before 1970 reads as 0.
fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs())
}
