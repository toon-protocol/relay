//! `POST /write`: where a paid write arrives. The connector settled the
//! payment before it delivered, so a request that reaches this handler is
//! already paid for; what is left is to verify the event, store it, and hand
//! it to the read side's open subscriptions.
//!
//! The statuses are the connector's contract (#185, stories 29 to 31): `200`
//! with the event id and the stored-at time, `400` for a body that is not a
//! delivery, `422` for an event that does not verify. A `200` also echoes the
//! payment the connector stated on the delivery, when it stated one (stories
//! 32 and 33); it is not persisted. The ephemeral lane is [`ephemeral`], and
//! is a separate endpoint on purpose; see [`payment`].

mod ephemeral;
mod limiter;
mod payment;

use std::time::Instant;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use nostr::event::Event;
use serde::Serialize;
use serde_json::Value;

pub(crate) use self::ephemeral::{Lane, write_ephemeral};
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

/// A request the relay turned away, small enough to travel in a `Result`.
struct Refusal {
    status: StatusCode,
    error: String,
}

impl IntoResponse for Refusal {
    fn into_response(self) -> Response {
        let Self { status, error } = self;
        (status, Json(Refused { error })).into_response()
    }
}

fn refusal(status: StatusCode, error: impl Into<String>) -> Refusal {
    Refusal {
        status,
        error: error.into(),
    }
}

fn refused(status: StatusCode, error: impl Into<String>) -> Response {
    refusal(status, error).into_response()
}

/// The body is read as bytes, not through a JSON extractor: the TypeScript
/// relay parses whatever arrives, whatever its `Content-Type`.
pub(crate) async fn write(State(relay): State<Relay>, headers: HeaderMap, body: Bytes) -> Response {
    let Ok(body) = serde_json::from_slice::<Value>(&body) else {
        return refused(StatusCode::BAD_REQUEST, "Invalid request body");
    };
    let event = match event_value(&body) {
        Ok(event) => event,
        Err(refusal) => return refusal.into_response(),
    };
    // Read before verification, so a write that goes on to fail is logged
    // with what the connector said about it.
    let payment = PaymentStatement::stated_on(&headers);
    if relay.log_writes {
        println!("{}", log_line(&event, "write", payment.as_ref()));
    }
    let event = match verified_event(&relay, event) {
        Ok(event) => event,
        Err(refusal) => return refusal.into_response(),
    };

    // An ephemeral event is paid for like any other, and delivered, never kept.
    if event.event().kind.is_ephemeral() {
        relay.read_side.deliver(&event);
    } else {
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
    }

    Json(Stored {
        event_id: event.event().id.to_hex(),
        stored_at: unix_seconds(),
        payment,
    })
    .into_response()
}

/// The `event` of a delivery's body, missing as the TypeScript relay reads
/// it: absent, or any value JavaScript calls falsy.
fn event_value(body: &Value) -> Result<Value, Refusal> {
    match body.get("event") {
        Some(event) if !is_falsy(event) => Ok(event.clone()),
        _ => Err(refusal(
            StatusCode::BAD_REQUEST,
            "Missing required field: event",
        )),
    }
}

/// The event's kind as the body states it, before it is known to be an event.
fn kind_of(event: &Value) -> Option<u64> {
    event.get("kind")?.as_u64()
}

/// The event in `event`, once its id and signature are proven, with the
/// verification timed for `GET /metrics`. Never validates a payment: that was
/// the connector's.
fn verified_event(relay: &Relay, event: Value) -> Result<VerifiedEvent, Refusal> {
    let event = serde_json::from_value::<Event>(event).map_err(|error| {
        refusal(
            StatusCode::UNPROCESSABLE_ENTITY,
            format!("Invalid event: {error}"),
        )
    })?;
    let started = Instant::now();
    let verified = VerifiedEvent::verify(event);
    relay.metrics.record_verify(started.elapsed());
    verified.map_err(|error| match error {
        RelayError::EventIdMismatch => {
            refusal(StatusCode::UNPROCESSABLE_ENTITY, "Invalid event id")
        }
        RelayError::EventSignatureInvalid => {
            refusal(StatusCode::UNPROCESSABLE_ENTITY, "Invalid event signature")
        }
        error => {
            eprintln!("write: an event could not be verified: {error}");
            refusal(
                StatusCode::INTERNAL_SERVER_ERROR,
                "The event could not be verified",
            )
        }
    })
}

/// The line logged per write when `TOON_LOG_WRITES=true`: the event id as
/// stated, the handler, and the payment the connector stated, if it did.
fn log_line(event: &Value, handler: &str, payment: Option<&PaymentStatement>) -> String {
    let id = event.get("id").and_then(Value::as_str).unwrap_or("");
    let attribution = payment.map_or_else(String::new, |payment| {
        attribution(payment.payer(), payment.amount(), payment.chain())
    });
    format!("[write] event={id} handler={handler}{attribution}")
}

/// The payment's part of a logged write. It takes the statement's parts, not
/// the statement, because only the paid-write handler may build one.
fn attribution(payer: &str, amount: &str, chain: Chain) -> String {
    format!(" payer={payer} amount={amount} chain={chain}")
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

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    const PAYER: &str = "evm:0xabababababababababababababababababababababababababababababababab";

    #[test]
    fn a_logged_write_names_the_payer_amount_and_chain_when_stated() {
        assert_eq!(
            attribution(PAYER, "10", Chain::Evm),
            format!(" payer={PAYER} amount=10 chain=evm")
        );
        assert_eq!(
            log_line(&json!({ "id": "abc" }), "write-ephemeral", None),
            "[write] event=abc handler=write-ephemeral"
        );
    }
}
