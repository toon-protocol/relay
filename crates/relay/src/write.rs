//! `POST /write`: where a paid write arrives. The connector settled the
//! payment before it delivered, so a request that reaches this handler is
//! already paid for; what is left is to verify the event, store it, and hand
//! it to the read side's open subscriptions.
//!
//! The statuses are the connector's contract (#185, stories 29 to 31): `200`
//! with the event id and the stored-at time, `400` for a body that is not a
//! delivery, `422` for an event that does not verify. A `200` also echoes the
//! payment the connector stated on the delivery, when it stated one (stories
//! 32 and 33); it is not persisted. The ephemeral lane (#198) is not built
//! yet, and is not built in this module: see [`payment`].

mod payment;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use nostr::event::Event;
use serde::Serialize;
use serde_json::Value;

pub use self::payment::{Chain, PaymentStatement};
use crate::clock::unix_seconds;
use crate::{Relay, RelayError, Saved, VerifiedEvent};

/// The `200` body.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Stored {
    event_id: String,
    /// Seconds since the Unix epoch.
    stored_at: u64,
    /// What the connector stated about the payment, when it stated it.
    #[serde(skip_serializing_if = "Option::is_none")]
    payment: Option<PaymentStatement>,
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
pub(crate) async fn write(State(relay): State<Relay>, headers: HeaderMap, body: Bytes) -> Response {
    let Ok(body) = serde_json::from_slice::<Value>(&body) else {
        return refused(StatusCode::BAD_REQUEST, "Invalid request body");
    };
    // Missing as the TypeScript relay reads it: absent, or any value
    // JavaScript calls falsy.
    let event = match body.get("event") {
        Some(event) if !is_falsy(event) => event.clone(),
        _ => return refused(StatusCode::BAD_REQUEST, "Missing required field: event"),
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
        Err(RelayError::EventSignatureInvalid) => {
            return refused(StatusCode::UNPROCESSABLE_ENTITY, "Invalid event signature");
        }
        Err(error) => {
            eprintln!("write: an event could not be verified: {error}");
            return refused(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The event could not be verified",
            );
        }
    };

    match relay.store.save(&event).await {
        Ok(Saved::New) => {
            // Stored either way, as the row is; served only while live.
            if relay.store.serves(event.event()) {
                relay.read_side.deliver(&event);
            }
        }
        // Already held, so already delivered: the connector retried. Or
        // blocked, or retracted: dropped without a word to the writer, who
        // paid and is answered as for any stored event.
        Ok(Saved::Duplicate | Saved::Dropped) => {}
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
        payment: PaymentStatement::stated_on(&headers),
    })
    .into_response()
}

/// Whether JavaScript's `!value` is true of a JSON value.
fn is_falsy(value: &Value) -> bool {
    match value {
        Value::Null => true,
        Value::Bool(flag) => !flag,
        Value::Number(number) => number.as_f64() == Some(0.0),
        Value::String(text) => text.is_empty(),
        Value::Array(_) | Value::Object(_) => false,
    }
}
