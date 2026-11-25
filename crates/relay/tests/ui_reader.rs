//! What an app UI's pages may show (#370): a page reads the store as the
//! anonymous reader, with what it may not see left out by the query itself,
//! so a page's limit counts only events it may show.
#![cfg(feature = "ui-social")]

mod common;

use std::collections::BTreeSet;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{delivery, running_with, signed_by, write};
use http_body_util::BodyExt;
use nostr::event::{Event, EventBuilder, FinalizeEvent, Kind, Tag};
use nostr::key::Keys;
use nostr::nips::nip19::ToBech32;
use nostr::types::Timestamp;
use tower::ServiceExt;

async fn get(relay: &relay::Relay, path: &str) -> (StatusCode, String) {
    let response = relay
        .read_router()
        .oneshot(Request::get(path).body(Body::empty()).expect("a request"))
        .await
        .expect("infallible");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

fn note(keys: &Keys, kind: u16, at: u64, content: &str, tags: &[&[&str]]) -> Event {
    EventBuilder::new(Kind::from(kind), content)
        .tags(
            tags.iter()
                .map(|tag| Tag::parse(tag.iter().copied()).expect("a tag")),
        )
        .custom_created_at(Timestamp::from(at))
        .finalize(keys)
        .expect("signs")
}

async fn store(relay: &relay::Relay, event: &Event) {
    assert_eq!(write(relay, delivery(event)).await.0, 200);
}

/// The numbers of the `public-note-<n>.` markers `page` shows.
fn public_notes(page: &str) -> BTreeSet<u32> {
    page.split("public-note-")
        .skip(1)
        .filter_map(|rest| rest.split('.').next()?.parse().ok())
        .collect()
}

#[tokio::test]
async fn a_feed_that_shows_forty_notes_shows_forty_public_ones_among_private_group_notes() {
    // `/social` shows the newest 40 posts of the 120 it asks for. Three of
    // every four notes here are in a private group, so a query that counted
    // them towards its limit would leave the page short of public notes.
    let running =
        running_with(|name| (name == "TOON_NIP29_GROUPS").then(|| "true".to_owned())).await;
    let relay = &running.relay;
    let owner = Keys::generate();
    let author = Keys::generate();
    store(
        relay,
        &signed_by(&owner, 9007, 10, &[&["h", "core"], &["private"]]),
    )
    .await;
    let now = Timestamp::now().as_secs() - 1_000;
    let mut public = 0;
    for i in 0..160 {
        let at = now + i;
        if i % 4 == 0 {
            let text = format!("public-note-{public}.");
            store(relay, &note(&author, 1, at, &text, &[])).await;
            public += 1;
        } else {
            let text = format!("members-only-{i}.");
            store(relay, &note(&owner, 1, at, &text, &[&["h", "core"]])).await;
        }
    }

    let (status, page) = get(relay, "/social").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!page.contains("members-only"), "{page}");
    assert_eq!(public_notes(&page), (0..40).collect(), "{page}");
}

#[tokio::test]
async fn a_gift_wrap_is_never_shown_on_any_page() {
    let running =
        running_with(|name| (name == "TOON_NIP17_RECIPIENT_ONLY").then(|| "true".to_owned())).await;
    let relay = &running.relay;
    let (author, recipient) = (Keys::generate(), Keys::generate());
    let at = Timestamp::now().as_secs() - 10;
    let recipient_hex = recipient.public_key().to_hex();
    let wrap = note(
        &author,
        1059,
        at,
        "wrapped-secret",
        &[&["p", &recipient_hex]],
    );
    store(relay, &wrap).await;

    let (status, page) = get(relay, &format!("/social/e/{}", wrap.id.to_hex())).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{page}");
    for path in [
        "/social".to_owned(),
        format!("/social/e/{}", wrap.id.to_hex()),
        format!(
            "/social/p/{}",
            author.public_key().to_bech32().expect("npub")
        ),
        format!(
            "/social/p/{}",
            recipient.public_key().to_bech32().expect("npub")
        ),
    ] {
        let (_, page) = get(relay, &path).await;
        assert!(!page.contains("wrapped-secret"), "{path}: {page}");
    }
}

#[tokio::test]
async fn a_kind_the_operator_restricts_is_never_shown_on_a_page() {
    let running =
        running_with(|name| (name == "TOON_AUTH_REQUIRED_KINDS").then(|| "1".to_owned())).await;
    let relay = &running.relay;
    let author = Keys::generate();
    let at = Timestamp::now().as_secs() - 10;
    let restricted = note(&author, 1, at, "restricted-note", &[]);
    store(relay, &restricted).await;

    let path = format!("/social/e/{}", restricted.id.to_hex());
    let (status, page) = get(relay, &path).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{page}");
    let (_, page) = get(relay, "/social").await;
    assert!(!page.contains("restricted-note"), "{page}");
}
