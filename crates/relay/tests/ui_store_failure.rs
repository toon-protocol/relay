//! A store that cannot answer is an error page on every app UI (#371), not
//! an empty page: a broken store must not look like a quiet relay.
#![cfg(all(feature = "ui-repo", feature = "ui-social", feature = "ui-compute"))]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::running_with;
use http_body_util::BodyExt;
use nostr::key::Keys;
use nostr::nips::nip19::ToBech32;
use tower::ServiceExt;

async fn get(relay: &relay::Relay, path: &str) -> (StatusCode, String, Option<String>) {
    let response = relay
        .read_router()
        .oneshot(Request::get(path).body(Body::empty()).expect("a request"))
        .await
        .expect("infallible");
    let status = response.status();
    let kind = response
        .headers()
        .get("content-type")
        .map(|value| value.to_str().expect("text").to_owned());
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned(), kind)
}

/// One page of each UI, and a preview image route of each.
fn paths() -> Vec<String> {
    let npub = Keys::generate().public_key().to_bech32().expect("npub");
    vec![
        "/repo".to_owned(),
        "/social".to_owned(),
        "/compute".to_owned(),
        format!("/repo/og/{npub}/demo.png"),
        format!("/social/og/p/{npub}.png"),
        format!("/compute/og/p/{npub}.png"),
    ]
}

#[tokio::test]
async fn a_page_answers_a_store_that_fails_every_query_with_an_error_page() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    for path in paths() {
        let (status, _, _) = get(relay, &path).await;
        assert!(
            status == StatusCode::OK || status == StatusCode::NOT_FOUND,
            "{path} answers a healthy store with {status}"
        );
    }

    // Another connection takes the table away: every query now fails.
    rusqlite::Connection::open(&running.database)
        .expect("the database opens")
        .execute_batch("ALTER TABLE events RENAME TO events_gone")
        .expect("the table is renamed");

    for path in paths() {
        let (status, body, kind) = get(relay, &path).await;
        assert!(
            status.is_server_error(),
            "{path} answers a failing store with {status}: {body}"
        );
        if path.ends_with(".png") {
            assert_ne!(kind.as_deref(), Some("image/png"), "{path} drew an image");
        } else {
            assert!(
                body.contains("The relay had an error"),
                "{path} is not the error page: {body}"
            );
        }
    }
}
