//! The read side over a real socket: the relay accepts the WebSocket upgrade
//! itself and the framework speaks NIP-01 on the stream it is handed.

mod common;

use axum::body::Body;
use axum::http::Request;
use common::{Client, delivery, running, running_with, signed, write};
use serde_json::json;
use tower::ServiceExt;

#[tokio::test]
async fn a_paid_write_is_returned_by_req_followed_by_eose() {
    let running = running().await;
    let event = signed(1, 1_700_000_000, &[&["t", "tracer"]]);
    let (status, _) = write(&running.relay, delivery(&event)).await;
    assert_eq!(status, 200);

    let mut client = Client::connect(&running.read_url).await;
    let found = client.req("stored", json!({ "kinds": [1] })).await;

    assert_eq!(
        found,
        vec![serde_json::to_value(&event).expect("an event is JSON")]
    );
}

#[tokio::test]
async fn a_paid_write_is_delivered_live_to_an_open_subscription() {
    let running = running().await;
    let mut client = Client::connect(&running.read_url).await;
    assert!(
        client
            .req("live", json!({ "kinds": [7777] }))
            .await
            .is_empty()
    );

    let event = signed(7777, 1_700_000_000, &[]);
    let other = signed(1, 1_700_000_000, &[]);
    for written in [&other, &event] {
        let (status, _) = write(&running.relay, delivery(written)).await;
        assert_eq!(status, 200);
    }

    assert_eq!(
        client.next().await,
        Some(json!(["EVENT", "live", event])),
        "the matching event arrives and the other does not"
    );
    assert_eq!(client.next().await, None);
}

#[tokio::test]
async fn an_event_refused_on_the_write_port_reaches_no_subscriber() {
    let running = running().await;
    let mut client = Client::connect(&running.read_url).await;
    assert!(client.req("live", json!({})).await.is_empty());

    let mut tampered = serde_json::to_value(signed(1, 1_700_000_000, &[])).expect("JSON");
    tampered["content"] = json!("tampered after signing");
    let (status, _) = write(&running.relay, json!({ "event": tampered }).to_string()).await;
    assert_eq!(status, 422);

    assert_eq!(client.next().await, None);
    assert!(client.req("stored", json!({})).await.is_empty());
}

#[tokio::test]
async fn event_over_websocket_is_refused_and_not_stored() {
    let running = running().await;
    let event = signed(1, 1_700_000_000, &[]);
    let mut client = Client::connect(&running.read_url).await;

    client.send(json!(["EVENT", event])).await;

    let frame = client.next().await.expect("an OK arrives");
    assert_eq!(frame[0], "OK");
    assert_eq!(frame[1], event.id.to_hex());
    assert_eq!(frame[2], false);
    let message = frame[3].as_str().expect("the refusal is text");
    assert!(
        message.starts_with("restricted: writes require ILP payment"),
        "{message}"
    );
    assert!(client.req("stored", json!({})).await.is_empty());
}

#[tokio::test]
async fn a_protected_or_expired_event_gets_the_same_refusal_and_no_auth() {
    let running = running().await;
    let mut client = Client::connect(&running.read_url).await;

    for event in [
        signed(1, 1_700_000_000, &[&["-"]]),
        signed(1, 1_700_000_000, &[&["expiration", "1"]]),
    ] {
        client.send(json!(["EVENT", event])).await;
        let frame = client.next().await.expect("an OK arrives");
        assert_eq!(frame[0], "OK", "no AUTH challenge comes first: {frame}");
        assert_eq!(frame[1], event.id.to_hex());
        assert_eq!(frame[2], false);
        let message = frame[3].as_str().expect("the refusal is text");
        assert!(
            message.starts_with("restricted: writes require ILP payment"),
            "{message}"
        );
    }
}

#[tokio::test]
async fn a_plain_get_on_the_read_port_is_426() {
    let running = running().await;
    let response = running
        .relay
        .read_router()
        .oneshot(Request::get("/").body(Body::empty()).expect("a valid GET"))
        .await
        .expect("the router is infallible");
    assert_eq!(response.status().as_u16(), 426);
}

#[tokio::test]
async fn the_subscription_after_the_limit_is_a_notice_with_no_eose() {
    let running = running().await;
    let mut client = Client::connect(&running.read_url).await;
    for i in 0..20 {
        assert!(
            client
                .req(&format!("s{i}"), no_such_event())
                .await
                .is_empty()
        );
    }

    client.send(json!(["REQ", "one-too-many", {}])).await;
    assert_eq!(
        client.next().await,
        Some(json!(["NOTICE", "error: too many subscriptions"]))
    );
    assert_eq!(client.next().await, None, "no EOSE follows the refusal");
    // Replacing a subscription is not a new one.
    assert!(client.req("s0", no_such_event()).await.is_empty());
}

#[tokio::test]
async fn a_request_with_too_many_filters_is_a_notice_with_no_eose() {
    let running = running().await;
    let mut client = Client::connect(&running.read_url).await;

    let mut message = vec![json!("REQ"), json!("many")];
    message.extend(std::iter::repeat_n(no_such_event(), 11));
    client.send(json!(message)).await;

    assert_eq!(
        client.next().await,
        Some(json!(["NOTICE", "error: too many filters"]))
    );
    assert_eq!(client.next().await, None);
}

#[tokio::test]
async fn an_empty_subscription_id_is_a_notice() {
    let running = running().await;
    let mut client = Client::connect(&running.read_url).await;
    client.send(json!(["REQ", "", {}])).await;
    assert_eq!(
        client.next().await,
        Some(json!(["NOTICE", "error: invalid subscription id"]))
    );
}

#[tokio::test]
async fn an_id_that_is_only_a_prefix_matches_nothing() {
    let running = running().await;
    let event = signed(1, 1_700_000_000, &[]);
    let (status, _) = write(&running.relay, delivery(&event)).await;
    assert_eq!(status, 200);
    let mut client = Client::connect(&running.read_url).await;

    let prefix = &event.id.to_hex()[..16];
    assert!(client.req("p", json!({ "ids": [prefix] })).await.is_empty());
    assert!(
        client
            .req("a", json!({ "authors": [&event.pubkey.to_hex()[..16]] }))
            .await
            .is_empty()
    );
    assert_eq!(
        client
            .req("whole", json!({ "ids": [event.id.to_hex()] }))
            .await
            .len(),
        1
    );
}

#[tokio::test]
async fn a_connection_past_the_cap_is_closed_with_1013() {
    use tokio_tungstenite::tungstenite::Message;
    use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;

    let running = running_with(|name| match name {
        "TOON_MAX_CONNECTIONS" => Some("2".to_string()),
        _ => None,
    })
    .await;
    let mut held = Vec::new();
    for _ in 0..2 {
        let mut client = Client::connect(&running.read_url).await;
        // A round trip proves the connection was admitted.
        assert!(client.req("hold", no_such_event()).await.is_empty());
        held.push(client);
    }

    let mut excess = Client::connect(&running.read_url).await;
    let Some(Message::Close(Some(frame))) = excess.next_message().await else {
        panic!("the excess connection is closed with a code");
    };
    assert_eq!(frame.code, CloseCode::Again);

    // A place frees when a connection ends.
    drop(held.pop());
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    let mut admitted = Client::connect(&running.read_url).await;
    assert!(admitted.req("again", no_such_event()).await.is_empty());
}

#[tokio::test]
async fn the_document_states_the_limits_and_does_not_advertise_auth() {
    let running = running().await;
    let response = running
        .relay
        .read_router()
        .oneshot(
            Request::get("/")
                .header("accept", "application/nostr+json")
                .body(Body::empty())
                .expect("a valid GET"),
        )
        .await
        .expect("the router is infallible");

    assert_eq!(response.status().as_u16(), 200);
    assert_eq!(response.headers()["access-control-allow-origin"], "*");
    assert_eq!(response.headers()["content-type"], "application/nostr+json");
    let body = http_body_util::BodyExt::collect(response.into_body())
        .await
        .expect("a body")
        .to_bytes();
    let document: serde_json::Value = serde_json::from_slice(&body).expect("JSON");
    assert_eq!(document["limitation"]["max_subscriptions"], 20);
    assert_eq!(document["limitation"]["max_filters"], 10);
    assert_eq!(document["limitation"]["max_limit"], 500);
    assert_eq!(document["limitation"]["default_limit"], 500);
    assert_eq!(document["limitation"]["auth_required"], false);
    // 40 is claimed while expiration is enforced, which it is by default.
    assert_eq!(document["supported_nips"], json!([1, 9, 11, 16, 40]));
}

/// A filter nothing matches, which a subscription can hold without the
/// framework closing it as unsatisfiable.
fn no_such_event() -> serde_json::Value {
    json!({ "ids": ["0".repeat(64)] })
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after the epoch")
        .as_secs()
}

#[tokio::test]
async fn an_expired_event_is_served_by_req_only_while_expiration_is_not_enforced() {
    let t = unix_now();
    let expired = signed(1, t - 200, &[&["expiration", &(t - 100).to_string()]]);
    let newer = signed(1, t - 50, &[]);
    let older = signed(1, t - 300, &[]);

    let lax = running_with(|name| match name {
        "TOON_ENFORCE_EXPIRATION" => Some("false".to_string()),
        _ => None,
    })
    .await;
    for event in [&older, &expired, &newer] {
        assert_eq!(write(&lax.relay, delivery(event)).await.0, 200);
    }
    let mut client = Client::connect(&lax.read_url).await;
    let found = client.req("all", json!({ "kinds": [1] })).await;
    let want = [&newer, &expired, &older].map(|e| serde_json::to_value(e).expect("JSON"));
    assert_eq!(found, want, "newest first, the expired one among them");

    let enforcing = running().await;
    for event in [&older, &newer] {
        assert_eq!(write(&enforcing.relay, delivery(event)).await.0, 200);
    }
    let mut client = Client::connect(&enforcing.read_url).await;
    let found = client.req("all", json!({ "kinds": [1] })).await;
    assert_eq!(found.len(), 2, "enforced: the expired event is not served");
}

#[tokio::test]
async fn a_multi_letter_tag_key_filters_stored_results_and_live_events_alike() {
    let t = unix_now();
    let hit = signed(1, t - 10, &[&["ab", "x"]]);
    let miss = signed(1, t - 20, &[&["ab", "y"]]);
    let running = running().await;
    for event in [&hit, &miss] {
        assert_eq!(write(&running.relay, delivery(event)).await.0, 200);
    }

    let mut client = Client::connect(&running.read_url).await;
    let found = client
        .req("multi", json!({ "kinds": [1], "#ab": ["x"] }))
        .await;
    assert_eq!(found, vec![serde_json::to_value(&hit).expect("JSON")]);

    let live_hit = signed(1, t - 5, &[&["ab", "x"]]);
    let live_miss = signed(1, t - 4, &[&["ab", "y"]]);
    for event in [&live_miss, &live_hit] {
        assert_eq!(write(&running.relay, delivery(event)).await.0, 200);
    }
    assert_eq!(
        client.next().await,
        Some(json!(["EVENT", "multi", live_hit]))
    );
    assert_eq!(client.next().await, None);
}
