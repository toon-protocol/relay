//! Link previews (#314, ADR 0006) over events written through `POST /write`,
//! driven through the router with no socket: a page names an absolute card
//! image, and that path answers a 1200×630 PNG drawn from the store.
#![cfg(all(feature = "ui-repo", feature = "ui-social"))]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use common::{delivery, running_with, write};
use http_body_util::BodyExt;
use nostr::event::{Event, EventBuilder, FinalizeEvent, Kind, Tag};
use nostr::key::Keys;
use nostr::nips::nip19::ToBech32;
use nostr::types::Timestamp;
use tower::ServiceExt;

const ORIGIN: &str = "https://relay.test";

struct Answer {
    status: StatusCode,
    headers: axum::http::HeaderMap,
    body: Vec<u8>,
}

async fn get(relay: &relay::Relay, path: &str, etag: Option<&str>) -> Answer {
    let mut request = Request::get(path);
    if let Some(etag) = etag {
        request = request.header(header::IF_NONE_MATCH, etag);
    }
    let response = relay
        .read_router()
        .oneshot(request.body(Body::empty()).expect("a request"))
        .await
        .expect("infallible");
    let status = response.status();
    let headers = response.headers().clone();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes()
        .to_vec();
    Answer {
        status,
        headers,
        body,
    }
}

fn event(keys: &Keys, kind: u16, at: u64, content: &str, tags: &[&[&str]]) -> Event {
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

/// The `content` of the `<meta>` named `name` in `html`.
fn meta(html: &str, name: &str) -> Option<String> {
    let needle = format!("=\"{name}\" content=\"");
    let start = html.find(&needle)? + needle.len();
    let rest = &html[start..];
    Some(rest[..rest.find('"')?].to_owned())
}

/// The width and height a PNG's header states, if it is a PNG.
fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
    if bytes.len() < 24 || &bytes[..8] != b"\x89PNG\r\n\x1a\n" || &bytes[12..16] != b"IHDR" {
        return None;
    }
    let word = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().expect("four bytes"));
    Some((word(16), word(20)))
}

/// `url` on [`ORIGIN`], as a path this relay answers.
fn path_of(url: &str) -> &str {
    url.strip_prefix(ORIGIN)
        .expect("an absolute URL on the public origin")
}

#[tokio::test]
async fn a_repo_page_names_an_absolute_card_that_is_a_png_of_the_open_graph_size() {
    let running =
        running_with(|name| (name == "TOON_UI_PUBLIC_URL").then(|| format!("{ORIGIN}/"))).await;
    let relay = &running.relay;
    let owner = Keys::generate();
    let fan = Keys::generate();
    let owner_hex = owner.public_key().to_hex();
    let npub = owner.public_key().to_bech32().expect("encodes");
    store(
        relay,
        &event(&owner, 0, 1, r#"{"name":"Ada","about":"Engines."}"#, &[]),
    )
    .await;
    store(
        relay,
        &event(
            &owner,
            30_617,
            2,
            "",
            &[
                &["d", "engine"],
                &["name", "engine"],
                &["description", "Computes <Bernoulli> numbers."],
                &["t", "math"],
            ],
        ),
    )
    .await;
    let coordinate = format!("30617:{owner_hex}:engine");
    store(
        relay,
        &event(&fan, 7, 3, "+", &[&["a", &coordinate], &["k", "30617"]]),
    )
    .await;

    for page in [
        format!("/social/r/{npub}/engine"),
        format!("/repo/{npub}/engine"),
    ] {
        let answer = get(relay, &page, None).await;
        assert_eq!(answer.status, StatusCode::OK, "{page}");
        let html = String::from_utf8(answer.body).expect("html");
        assert_eq!(
            meta(&html, "og:title").as_deref(),
            Some("Ada / engine"),
            "{page}"
        );
        assert_eq!(
            meta(&html, "og:description").as_deref(),
            Some("Computes &lt;Bernoulli&gt; numbers."),
            "{page}"
        );
        assert_eq!(
            meta(&html, "twitter:card").as_deref(),
            Some("summary_large_image")
        );
        assert_eq!(meta(&html, "og:url"), Some(format!("{ORIGIN}{page}")));
        // Both apps name the one card, counted as /social counts.
        let image = meta(&html, "og:image").expect("an og:image");
        assert_eq!(
            image,
            format!("{ORIGIN}/social/og/r/{npub}/engine.png"),
            "{page}"
        );
        assert_eq!(meta(&html, "twitter:image").as_ref(), Some(&image));

        let card = get(relay, path_of(&image), None).await;
        assert_eq!(card.status, StatusCode::OK);
        assert_eq!(card.headers[header::CONTENT_TYPE], "image/png");
        assert_eq!(png_size(&card.body), Some((1200, 630)));
        let etag = card.headers[header::ETAG]
            .to_str()
            .expect("text")
            .to_owned();
        let again = get(relay, path_of(&image), Some(&etag)).await;
        assert_eq!(again.status, StatusCode::NOT_MODIFIED);
    }

    // A second star is a different picture, so a different ETag.
    let first = get(relay, &format!("/social/og/r/{npub}/engine.png"), None).await;
    store(
        relay,
        &event(
            &Keys::generate(),
            7,
            4,
            "+",
            &[&["a", &coordinate], &["k", "30617"]],
        ),
    )
    .await;
    let second = get(relay, &format!("/social/og/r/{npub}/engine.png"), None).await;
    assert_ne!(first.headers[header::ETAG], second.headers[header::ETAG]);

    // /repo's own card, for a relay serving /repo alone.
    let own = get(relay, &format!("/repo/og/{npub}/engine.png"), None).await;
    assert_eq!(png_size(&own.body), Some((1200, 630)));

    // The profile card, and unknown things have none.
    let profile_html =
        String::from_utf8(get(relay, &format!("/social/p/{npub}"), None).await.body).expect("html");
    assert_eq!(meta(&profile_html, "og:type").as_deref(), Some("profile"));
    assert_eq!(
        meta(&profile_html, "og:description").as_deref(),
        Some("Engines.")
    );
    let profile_image = meta(&profile_html, "og:image").expect("an og:image");
    assert_eq!(profile_image, format!("{ORIGIN}/social/og/p/{npub}.png"));
    let profile = get(relay, path_of(&profile_image), None).await;
    assert_eq!(png_size(&profile.body), Some((1200, 630)));

    let stranger = Keys::generate().public_key().to_bech32().expect("encodes");
    for missing in [
        format!("/social/og/r/{npub}/nothing.png"),
        format!("/social/og/p/{stranger}.png"),
        format!("/repo/og/{npub}/nothing.png"),
        format!("/social/og/e/{}.png", "0".repeat(64)),
        format!("/social/og/r/{npub}/engine.gif"),
    ] {
        assert_eq!(
            get(relay, &missing, None).await.status,
            StatusCode::NOT_FOUND,
            "{missing}"
        );
    }

    // A page without a card of its own names the site card.
    let feed = String::from_utf8(get(relay, "/social/trending", None).await.body).expect("html");
    let site = meta(&feed, "og:image").expect("the site card");
    assert_eq!(site, format!("{ORIGIN}/social/og/site.png"));
    assert_eq!(
        png_size(&get(relay, path_of(&site), None).await.body),
        Some((1200, 630))
    );
}

#[tokio::test]
async fn without_a_public_origin_a_page_names_no_image() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let owner = Keys::generate();
    let npub = owner.public_key().to_bech32().expect("encodes");
    store(relay, &event(&owner, 30_617, 1, "", &[&["d", "engine"]])).await;
    let html = String::from_utf8(
        get(relay, &format!("/social/r/{npub}/engine"), None)
            .await
            .body,
    )
    .expect("html");
    assert!(meta(&html, "og:title").is_some());
    assert_eq!(meta(&html, "og:image"), None);
    assert_eq!(meta(&html, "og:url"), None);
    assert_eq!(meta(&html, "twitter:card").as_deref(), Some("summary"));
    // The card itself is still served: only the page cannot name it.
    let card = get(relay, &format!("/social/og/r/{npub}/engine.png"), None).await;
    assert_eq!(card.status, StatusCode::OK);
}

#[tokio::test]
async fn a_secure_relay_url_stands_in_for_the_public_origin() {
    let running =
        running_with(|name| (name == "TOON_UI_RELAY_URL").then(|| "wss://relay.test".to_owned()))
            .await;
    let html = String::from_utf8(get(&running.relay, "/social", None).await.body).expect("html");
    assert_eq!(
        meta(&html, "og:image").as_deref(),
        Some("https://relay.test/social/og/site.png")
    );
}

#[test]
fn a_public_url_that_is_not_an_https_origin_is_refused() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let data_dir = dir.path().to_string_lossy().into_owned();
    for bad in [
        "http://relay.test",
        "https://relay.test/social",
        "relay.test",
    ] {
        let refused = relay::Config::from_env(|name| match name {
            "TOON_SECRET_KEY" => Some("1".repeat(64)),
            "TOON_DATA_DIR" => Some(data_dir.clone()),
            "TOON_UI_PUBLIC_URL" => Some(bad.to_owned()),
            _ => None,
        });
        assert!(refused.is_err(), "{bad}");
    }
}
