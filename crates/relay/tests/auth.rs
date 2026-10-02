//! NIP-42 over a real socket (#218): off unless the operator turns it on, a
//! challenge for every connection, and reads of chosen kinds that wait for an
//! `AUTH`.

mod common;

use axum::body::Body;
use axum::http::Request;
use common::{Client, delivery, running, running_with, signed, signed_by, write};
use http_body_util::BodyExt;
use nostr::key::Keys;
use serde_json::{Value, json};
use tower::ServiceExt;

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after the epoch")
        .as_secs()
}

async fn with_auth(setting: &'static str, value: &'static str) -> common::Running {
    running_with(move |name| (name == setting).then(|| value.to_string())).await
}

/// The challenge a connection is sent as it opens.
async fn challenge(client: &mut Client) -> String {
    let frame = client.next().await.expect("a challenge arrives first");
    assert_eq!(frame[0], "AUTH", "{frame}");
    frame[1].as_str().expect("a challenge is text").to_string()
}

fn answer(keys: &Keys, challenge: &str, at: u64) -> nostr::event::Event {
    signed_by(
        keys,
        22242,
        at,
        &[&["relay", "ws://relay.test"], &["challenge", challenge]],
    )
}

async fn supported_nips(running: &common::Running) -> Value {
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
    let body = response
        .into_body()
        .collect()
        .await
        .expect("a body")
        .to_bytes();
    let document: Value = serde_json::from_slice(&body).expect("the document is JSON");
    document["supported_nips"].clone()
}

#[tokio::test]
async fn by_default_no_challenge_is_sent_nothing_is_restricted_and_42_is_not_listed() {
    let running = running().await;
    let event = signed(4, 1_700_000_000, &[]);
    assert_eq!(write(&running.relay, delivery(&event)).await.0, 200);

    let mut client = Client::connect(&running.read_url).await;
    let found = client.req("dm", json!({ "kinds": [4] })).await;
    assert_eq!(found, vec![serde_json::to_value(&event).expect("JSON")]);
    assert_eq!(supported_nips(&running).await, json!([1, 9, 11, 16, 40]));
}

#[tokio::test]
async fn an_enabled_relay_lists_42_and_challenges_each_connection_differently() {
    let running = with_auth("TOON_NIP42_AUTH", "true").await;
    assert_eq!(
        supported_nips(&running).await,
        json!([1, 9, 11, 16, 40, 42])
    );

    let mut first = Client::connect(&running.read_url).await;
    let mut second = Client::connect(&running.read_url).await;
    assert_ne!(challenge(&mut first).await, challenge(&mut second).await);
}

#[tokio::test]
async fn enabled_with_no_kinds_chosen_nothing_is_restricted() {
    let running = with_auth("TOON_NIP42_AUTH", "true").await;
    let mut client = Client::connect(&running.read_url).await;
    challenge(&mut client).await;
    assert!(client.req("all", json!({})).await.is_empty());
}

#[tokio::test]
async fn a_valid_auth_is_accepted_and_one_for_another_challenge_or_time_is_not() {
    let running = with_auth("TOON_NIP42_AUTH", "true").await;
    let mut client = Client::connect(&running.read_url).await;
    let issued = challenge(&mut client).await;
    let keys = Keys::generate();
    let now = unix_now();

    for (event, accepted) in [
        (answer(&keys, "never issued", now), false),
        (answer(&keys, &issued, now - 3600), false),
        (answer(&keys, &issued, now), true),
    ] {
        client.send(json!(["AUTH", event])).await;
        let frame = client.next().await.expect("an OK arrives");
        assert_eq!(
            (&frame[0], &frame[1], &frame[2]),
            (&json!("OK"), &json!(event.id), &json!(accepted)),
            "{frame}"
        );
    }
}

#[tokio::test]
async fn chosen_kinds_are_closed_auth_required_until_the_connection_answers() {
    let running = with_auth("TOON_AUTH_REQUIRED_KINDS", "4,1059").await;
    let private = signed(4, 1_700_000_000, &[]);
    assert_eq!(write(&running.relay, delivery(&private)).await.0, 200);
    assert_eq!(
        supported_nips(&running).await,
        json!([1, 9, 11, 16, 40, 42])
    );

    let mut client = Client::connect(&running.read_url).await;
    let issued = challenge(&mut client).await;

    for (id, filter) in [
        ("named", json!({ "kinds": [4] })),
        ("mixed", json!({ "kinds": [1, 1059] })),
        ("unnamed", json!({})),
    ] {
        client.send(json!(["REQ", id, filter])).await;
        let frame = client.next().await.expect("a CLOSED arrives");
        assert_eq!(
            (&frame[0], &frame[1]),
            (&json!("CLOSED"), &json!(id)),
            "{frame}"
        );
        assert!(
            frame[2]
                .as_str()
                .is_some_and(|text| text.starts_with("auth-required:"))
        );
    }
    assert!(
        client
            .req("notes", json!({ "kinds": [1] }))
            .await
            .is_empty(),
        "a kind that is not chosen is answered"
    );

    let auth = answer(&Keys::generate(), &issued, unix_now());
    client.send(json!(["AUTH", auth])).await;
    let frame = client.next().await.expect("an OK arrives");
    assert_eq!(frame, json!(["OK", auth.id, true, ""]));

    let found = client.req("named", json!({ "kinds": [4] })).await;
    assert_eq!(found, vec![serde_json::to_value(&private).expect("JSON")]);
}

#[tokio::test]
async fn authenticating_on_one_connection_opens_nothing_on_another() {
    let running = with_auth("TOON_AUTH_REQUIRED_KINDS", "4").await;
    let mut first = Client::connect(&running.read_url).await;
    let mut second = Client::connect(&running.read_url).await;
    let issued = challenge(&mut first).await;
    challenge(&mut second).await;

    let auth = answer(&Keys::generate(), &issued, unix_now());
    first.send(json!(["AUTH", auth])).await;
    assert_eq!(first.next().await.expect("an OK")[2], true);

    second.send(json!(["REQ", "dm", { "kinds": [4] }])).await;
    let frame = second.next().await.expect("a CLOSED arrives");
    assert_eq!(frame[0], "CLOSED", "{frame}");
}
