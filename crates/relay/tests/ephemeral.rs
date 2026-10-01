//! `POST /write-ephemeral`: verified, delivered, never stored, and bounded.

mod common;

use axum::body::Body;
use axum::http::Request;
use common::{Client, delivery, running, signed};
use http_body_util::BodyExt;
use serde_json::json;
use tower::ServiceExt;

async fn post(relay: &relay::Relay, body: impl Into<String>) -> (u16, serde_json::Value) {
    let request = Request::post("/write-ephemeral")
        .header("content-type", "application/json")
        .body(Body::from(body.into()))
        .expect("a POST with a string body is a valid request");
    let response = relay
        .write_router()
        .oneshot(request)
        .await
        .expect("the router is infallible");
    let status = response.status().as_u16();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, serde_json::from_slice(&body).expect("JSON"))
}

#[tokio::test]
async fn an_ephemeral_event_reaches_a_live_subscriber_and_is_not_stored() {
    let running = running().await;
    let mut client = Client::connect(&running.read_url).await;
    assert!(
        client
            .req("live", json!({ "kinds": [20100] }))
            .await
            .is_empty()
    );
    let event = signed(20100, 1_700_000_000, &[]);

    let (status, answer) = post(&running.relay, delivery(&event)).await;

    assert_eq!(status, 200);
    assert_eq!(answer["eventId"], event.id.to_hex());
    assert!(answer["broadcastAt"].is_u64());
    let frame = client.next().await.expect("the event is delivered");
    assert_eq!(frame[0], "EVENT");
    assert_eq!(frame[2]["id"], event.id.to_hex());
    let mut other = Client::connect(&running.read_url).await;
    assert!(
        other
            .req("history", json!({ "ids": [event.id.to_hex()] }))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn a_paid_ephemeral_write_is_delivered_and_not_stored() {
    let running = running().await;
    let mut client = Client::connect(&running.read_url).await;
    client.req("live", json!({ "kinds": [20101] })).await;
    let event = signed(20101, 1_700_000_000, &[]);

    let (status, _) = common::write(&running.relay, delivery(&event)).await;

    assert_eq!(status, 200);
    assert_eq!(
        client.next().await.expect("delivered")[2]["id"],
        event.id.to_hex()
    );
    let mut other = Client::connect(&running.read_url).await;
    assert!(
        other
            .req("history", json!({ "ids": [event.id.to_hex()] }))
            .await
            .is_empty()
    );
}

#[tokio::test]
async fn a_kind_outside_the_ephemeral_range_a_missing_event_or_a_non_json_body_is_400() {
    let running = running().await;
    for body in [
        delivery(&signed(1, 1_700_000_000, &[])),
        delivery(&signed(30000, 1_700_000_000, &[])),
        "{}".to_string(),
        "not json".to_string(),
    ] {
        assert_eq!(post(&running.relay, body).await.0, 400);
    }
}

#[tokio::test]
async fn a_bad_signature_is_422() {
    let running = running().await;
    let mut event = serde_json::to_value(signed(20100, 1_700_000_000, &[])).unwrap();
    event["sig"] = json!("0".repeat(128));
    let (status, _) = post(&running.relay, json!({ "event": event }).to_string()).await;
    assert_eq!(status, 422);
}

#[tokio::test]
async fn a_body_over_8_kib_is_413() {
    let running = running().await;
    let mut event = serde_json::to_value(signed(20100, 1_700_000_000, &[])).unwrap();
    event["content"] = json!("x".repeat(16 * 1024));
    let (status, _) = post(&running.relay, json!({ "event": event }).to_string()).await;
    assert_eq!(status, 413);
}

#[tokio::test]
async fn a_caller_over_200_requests_in_the_window_is_429() {
    let running = running().await;
    let mut statuses = Vec::new();
    for _ in 0..201 {
        statuses.push(post(&running.relay, "{}").await.0);
    }
    assert!(statuses[..200].iter().all(|status| *status == 400));
    assert_eq!(statuses[200], 429);
}
