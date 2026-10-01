//! `POST /write`: what the connector is told about the event it delivered.
//! Driven through the router, with no socket.

mod common;

use std::time::{SystemTime, UNIX_EPOCH};

use common::{delivery, running, signed, write};
use serde_json::json;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the test host's clock is after 1970")
        .as_secs()
}

#[tokio::test]
async fn a_signed_regular_event_is_answered_200_with_its_id_and_a_stored_at_time() {
    let running = running().await;
    let event = signed(1, 1_700_000_000, &[]);
    let before = now();

    let (status, body) = write(&running.relay, delivery(&event)).await;

    assert_eq!(status, 200);
    let object = body.as_object().expect("the body is an object");
    let mut keys: Vec<_> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["eventId", "storedAt"]);
    assert_eq!(body["eventId"], event.id.to_hex());
    let stored_at = body["storedAt"].as_u64().expect("whole seconds");
    assert!((before..=now()).contains(&stored_at));
}

#[tokio::test]
async fn a_body_that_is_not_json_or_carries_no_event_is_400() {
    let running = running().await;
    let bodies = [
        "not json",
        "{}",
        "[]",
        r#"{"event":null}"#,
        r#"{"event":false}"#,
        r#"{"event":0}"#,
        r#"{"event":""}"#,
    ];
    for body in bodies {
        let (status, answer) = write(&running.relay, body).await;
        assert_eq!(status, 400, "{body}");
        assert!(answer["error"].is_string(), "{body}");
    }
}

#[tokio::test]
async fn a_bad_signature_or_a_tampered_event_is_422() {
    let running = running().await;
    let event = serde_json::to_value(signed(1, 1_700_000_000, &[])).expect("an event is JSON");

    let mut zero_signature = event.clone();
    zero_signature["sig"] = json!("0".repeat(128));
    let mut tampered = event.clone();
    tampered["content"] = json!("tampered after signing");
    let not_an_event = json!({ "id": "abc" });

    for bad in [zero_signature, tampered, not_an_event] {
        let (status, answer) = write(&running.relay, json!({ "event": bad }).to_string()).await;
        assert_eq!(status, 422, "{bad}");
        assert!(answer["error"].is_string(), "{bad}");
    }
}

#[tokio::test]
async fn a_kind_whose_storage_rule_is_not_built_yet_is_501() {
    let running = running().await;
    let profile = signed(0, 1_700_000_000, &[]);

    let (status, answer) = write(&running.relay, delivery(&profile)).await;

    assert_eq!(status, 501);
    assert_eq!(
        answer["error"],
        "events of kind 0 are not stored by this build yet"
    );
}
