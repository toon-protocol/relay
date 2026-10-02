//! The paid live feed's HTTP surfaces (#215, the draft NIP paid-subscription):
//! the subscribe route, the balance read, and the operator's list.
//!
//! - `POST /subscribe` on the write port is where the connector delivers a
//!   packet paid at the subscribe route. It is the write port's, not the read
//!   port's, for the reason `POST /write` is: nothing but the relay's own
//!   connector can reach it, so a request here is already paid for. What a
//!   packet credits is what the route charged: the amount the connector
//!   states it charged, else the route's flat price from its self-description.
//!   The payer the connector may state is not read.
//! - `GET /` on the read port with `Accept: application/toon-subscription+json`
//!   is the subscriber reading its own balance, free, under NIP-98.
//! - `GET /subscribers` on the write port is the operator's list. The write
//!   port is the operator's surface (`/metrics` is there too) and is not
//!   published; the list names keys and balances, so it must not be.
//!
//! A refusal is `{ "error": { "code", "message" } }`. A connector fulfils a
//! packet whatever the app answered, so a refusal here still costs its price:
//! the draft says so and tells subscribers to check before they pay.

use std::collections::HashSet;
use std::sync::Arc;

use axum::Json;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use nostr::key::PublicKey;
use serde::Serialize;
use serde_json::{Value, json};

use crate::Relay;
use crate::clock::unix_seconds;
use crate::connector::OfferSlot;
use crate::ledger::{CreditError, Ledger, Snapshot};
use crate::proof::{Hosts, http_authorization};
use crate::session::Wanted;
use crate::write::charged_amount;

/// The media type a balance is read as and answered in.
pub(crate) const BALANCE_MEDIA: &str = "application/toon-subscription+json";

/// What a relay that sells its feed holds besides the store: the books, who
/// it is, who follows for nothing, and the subscribe route's offer.
#[derive(Debug, Clone)]
pub(crate) struct Sale {
    pub(crate) ledger: Ledger,
    /// How the relay names itself in a NIP-42 and a NIP-98 event.
    pub(crate) hosts: Hosts,
    /// The keys that read the live feed for nothing: the relay's own, and
    /// those `TOON_OPERATOR_PUBKEYS` names.
    pub(crate) operators: Arc<HashSet<PublicKey>>,
    pub(crate) offer: OfferSlot,
}

fn refusal(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(json!({ "error": { "code": code, "message": message } })),
    )
        .into_response()
}

fn unauthorized(message: &str) -> Response {
    refusal(StatusCode::UNAUTHORIZED, "unauthorized", message)
}

/// The subscription as the draft's `200` bodies carry it.
#[derive(Serialize)]
struct Standing {
    pubkey: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    credited: Option<u64>,
    balance: u64,
    broadcast_price: u64,
    filter: Value,
}

impl Standing {
    fn of(snapshot: Snapshot, broadcast_price: u64, credited: Option<u64>) -> Self {
        Self {
            pubkey: snapshot.pubkey.to_hex(),
            credited,
            balance: snapshot.balance,
            broadcast_price,
            filter: snapshot.filter,
        }
    }
}

/// `POST /subscribe`: one paid packet at the subscribe route.
pub(crate) async fn subscribe(
    State(relay): State<Relay>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(sale) = &relay.sale else {
        return refusal(
            StatusCode::NOT_FOUND,
            "not_found",
            "this relay does not sell its live feed",
        );
    };
    let key = match http_authorization(&headers, "POST", Some(&body), &sale.hosts, unix_seconds()) {
        Ok(key) => key,
        Err(message) => return unauthorized(message),
    };
    let invalid = |message: &str| refusal(StatusCode::BAD_REQUEST, "invalid_request", message);
    let Ok(Value::Object(request)) = serde_json::from_slice::<Value>(&body) else {
        return invalid("the body must be a JSON object");
    };
    let filter = match request.get("filter") {
        None => None,
        Some(filter) => match (filter.is_object(), Wanted::read(filter.clone())) {
            (true, Some(wanted)) => Some((filter.clone(), wanted)),
            _ => return invalid("filter must be one NIP-01 filter"),
        },
    };
    let filter_required = || {
        refusal(
            StatusCode::BAD_REQUEST,
            "filter_required",
            "a first payment needs a filter",
        )
    };
    if filter.is_none() && sale.ledger.subscription(&key).is_none() {
        return filter_required();
    }
    let Some(credited) =
        charged_amount(&headers).or_else(|| sale.offer.current().map(|o| o.price()))
    else {
        return refusal(
            StatusCode::SERVICE_UNAVAILABLE,
            "route_unknown",
            "the relay cannot tell what this packet paid: its connector states no amount and \
             publishes no price for the route",
        );
    };
    match sale.ledger.credit(key, credited, filter).await {
        Ok(snapshot) => {
            let price = sale.ledger.broadcast_price();
            Json(Standing::of(snapshot, price, Some(credited))).into_response()
        }
        Err(CreditError::FilterRequired) => filter_required(),
        Err(CreditError::NotKept(reason)) => {
            eprintln!("subscribe: a credit of {credited} to {key} was not kept: {reason}");
            refusal(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                "the credit could not be recorded",
            )
        }
    }
}

/// A subscriber reading its own subscription: `GET` with the media type, under
/// NIP-98. `None` when the relay does not sell its feed, so the request is
/// answered as any other on the read port.
pub(crate) fn balance(relay: &Relay, headers: &HeaderMap) -> Option<Response> {
    let sale = relay.sale.as_ref()?;
    let key = match http_authorization(headers, "GET", None, &sale.hosts, unix_seconds()) {
        Ok(key) => key,
        Err(message) => return Some(unauthorized(message)),
    };
    let Some(snapshot) = sale.ledger.subscription(&key) else {
        return Some(refusal(
            StatusCode::NOT_FOUND,
            "not_subscribed",
            "this key has no subscription at this relay",
        ));
    };
    let price = sale.ledger.broadcast_price();
    let mut standing = serde_json::to_value(Standing::of(snapshot, price, None))
        .expect("a subscription is strings and numbers, which always serialize");
    // The read answers with the four fields the draft names.
    if let Some(fields) = standing.as_object_mut() {
        fields.remove("credited");
    }
    Some(
        (
            StatusCode::OK,
            [(CONTENT_TYPE, BALANCE_MEDIA)],
            Json(standing),
        )
            .into_response(),
    )
}

/// `GET /subscribers`: every subscription and its balance, for the operator.
pub(crate) async fn subscribers(State(relay): State<Relay>) -> Response {
    let Some(sale) = &relay.sale else {
        return refusal(
            StatusCode::NOT_FOUND,
            "not_found",
            "this relay does not sell its live feed",
        );
    };
    let price = sale.ledger.broadcast_price();
    let subscribers: Vec<Standing> = sale
        .ledger
        .subscriptions()
        .into_iter()
        .map(|snapshot| Standing::of(snapshot, price, None))
        .collect();
    Json(json!({ "broadcast_price": price, "subscribers": subscribers })).into_response()
}
