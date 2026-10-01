//! The read side over a real socket: the relay accepts the WebSocket upgrade
//! itself and the framework speaks NIP-01 on the stream it is handed.

mod common;

use axum::body::Body;
use axum::http::Request;
use common::{Client, delivery, running, signed, write};
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
