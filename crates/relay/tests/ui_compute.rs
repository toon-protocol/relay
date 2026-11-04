//! The `/compute` pages over events written through `POST /write`, driven
//! through the router with no socket: the Provider Directory, its site bar
//! link, and the app being off.
#![cfg(all(feature = "ui-repo", feature = "ui-social", feature = "ui-compute"))]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{delivery, running_with, write};
use http_body_util::BodyExt;
use nostr::event::{Event, EventBuilder, FinalizeEvent, Kind, Tag};
use nostr::key::Keys;
use tower::ServiceExt;

const POLICY: &str = "default-src 'self'; script-src 'none'; img-src 'self'; \
                      form-action 'self'; base-uri 'none'";

struct Page {
    status: StatusCode,
    policy: Option<String>,
    text: String,
}

async fn get(relay: &relay::Relay, path: &str) -> Page {
    let response = relay
        .read_router()
        .oneshot(Request::get(path).body(Body::empty()).expect("a request"))
        .await
        .expect("infallible");
    let (status, policy) = (
        response.status(),
        response
            .headers()
            .get("content-security-policy")
            .map(|value| value.to_str().expect("text").to_owned()),
    );
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    Page {
        status,
        policy,
        text: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn event(keys: &Keys, kind: u16, content: &str, tags: &[&[&str]]) -> Event {
    EventBuilder::new(Kind::from(kind), content)
        .tags(
            tags.iter()
                .map(|tag| Tag::parse(tag.iter().copied()).expect("a tag")),
        )
        .finalize(keys)
        .expect("signs")
}

async fn store(relay: &relay::Relay, event: &Event) {
    assert_eq!(write(relay, delivery(event)).await.0, 200);
}

async fn listing(relay: &relay::Relay, keys: &Keys, d: &str, content: &str) {
    let a = format!("10432:{}:", keys.public_key().to_hex());
    store(
        relay,
        &event(
            keys,
            30_432,
            content,
            &[&["d", d], &["a", &a], &["L", "toon.network"]],
        ),
    )
    .await;
}

#[tokio::test]
async fn compute_lists_listings_whose_profile_the_relay_holds() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let (provider, orphan) = (Keys::generate(), Keys::generate());
    store(
        relay,
        &event(
            &provider,
            10_432,
            r#"{"name":"Tidal <b>Metal</b>"}"#,
            &[&["L", "toon.network"]],
        ),
    )
    .await;
    listing(
        relay,
        &provider,
        "small",
        r#"{"name":"Small","resources":{"vcpu":2,"memory":"4 GiB"},"arch":"arm64"}"#,
    )
    .await;
    listing(
        relay,
        &provider,
        "large",
        r#"{"name":"Large","resources":{"vcpu":8,"memory":"32 GiB"},"arch":"x86_64"}"#,
    )
    .await;
    listing(relay, &orphan, "lost", r#"{"name":"Orphaned"}"#).await;
    // Not a TOON Compute event: no `["L","toon.network"]`.
    let a = format!("10432:{}:", provider.public_key().to_hex());
    store(
        relay,
        &event(
            &provider,
            30_432,
            r#"{"name":"Unlabelled"}"#,
            &[&["d", "unlabelled"], &["a", &a]],
        ),
    )
    .await;

    let page = get(relay, "/compute").await;
    assert_eq!(page.status, StatusCode::OK);
    assert_eq!(page.policy.as_deref(), Some(POLICY));
    assert!(!page.text.contains("<script"));
    for shown in ["Small", "Large", "vcpu 2", "32 GiB", "arm64", "x86_64"] {
        assert!(page.text.contains(shown), "{shown}: {}", page.text);
    }
    assert!(page.text.contains("Tidal &lt;b&gt;Metal&lt;/b&gt;"));
    assert!(!page.text.contains("Tidal <b>"));
    assert!(!page.text.contains("Orphaned"));
    assert!(!page.text.contains("Unlabelled"));
    assert!(
        page.text
            .contains("only Providers that publish to this relay")
    );
    assert!(
        page.text
            .contains("class=\"navlink current\" href=\"/compute\"")
    );
}

#[tokio::test]
async fn the_site_bar_links_to_compute_on_every_app() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let owner = Keys::generate();
    store(relay, &event(&owner, 30_617, "", &[&["d", "r"]])).await;
    for path in [
        "/repo".to_owned(),
        "/social".to_owned(),
        "/compute".to_owned(),
    ] {
        let page = get(relay, &path).await;
        assert_eq!(page.status, StatusCode::OK, "{path}");
        assert!(page.text.contains("href=\"/compute\""), "{path}");
        assert!(page.text.contains(">Compute</a>"), "{path}");
    }
}

#[tokio::test]
async fn with_compute_off_it_is_426_and_no_page_links_to_it() {
    let running =
        running_with(|name| (name == "TOON_UI_APPS").then(|| "repo,social".to_owned())).await;
    let relay = &running.relay;
    assert_eq!(
        get(relay, "/compute").await.status,
        StatusCode::UPGRADE_REQUIRED
    );
    for path in ["/repo", "/social"] {
        let page = get(relay, path).await;
        assert_eq!(page.status, StatusCode::OK, "{path}");
        assert!(!page.text.contains("/compute"), "{path}");
    }
}

#[tokio::test]
async fn an_unknown_path_under_compute_is_the_404_page() {
    let running = running_with(|_| None).await;
    for path in ["/compute/nope", "/compute/p/none/x/y"] {
        let page = get(&running.relay, path).await;
        assert_eq!(page.status, StatusCode::NOT_FOUND, "{path}");
        assert_eq!(page.policy.as_deref(), Some(POLICY), "{path}");
        assert!(page.text.contains("Not found"), "{path}");
    }
}
