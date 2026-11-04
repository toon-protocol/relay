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
async fn compute_shows_live_status_price_and_standby_live_and_cheapest_first() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let (up, down) = (Keys::generate(), Keys::generate());
    for keys in [&up, &down] {
        store(relay, &event(keys, 10_432, "{}", &[&["L", "toon.network"]])).await;
    }
    listing(
        relay,
        &down,
        "cheap-down",
        r#"{"name":"CheapDown","price":1,"lease_interval_s":3600}"#,
    )
    .await;
    listing(
        relay,
        &up,
        "dear-up",
        r#"{"name":"DearUp","price":1000,"lease_interval_s":7,"standby_price":40}"#,
    )
    .await;
    listing(
        relay,
        &up,
        "cheap-up",
        r#"{"name":"CheapUp","price":50,"lease_interval_s":60}"#,
    )
    .await;
    let later = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after the epoch")
        .as_secs()
        + 3_600)
        .to_string();
    store(
        relay,
        &event(
            &up,
            10_433,
            r#"{"available":{"cheap-up":3}}"#,
            &[&["L", "toon.network"], &["expiration", &later]],
        ),
    )
    .await;

    let page = get(relay, "/compute").await;
    assert_eq!(page.status, StatusCode::OK);
    let at = |name: &str| {
        page.text
            .find(&format!("data-listing=\"{name}\""))
            .unwrap_or_else(|| panic!("{name}: {}", page.text))
    };
    assert!(at("CheapUp") < at("DearUp"));
    assert!(at("DearUp") < at("CheapDown"));
    let row = |name: &str| {
        let rest = &page.text[at(name)..];
        rest[..rest.find("</tr>").expect("a row end")].to_owned()
    };
    let cheap_up = row("CheapUp");
    assert!(cheap_up.contains("live, 3 available"), "{cheap_up}");
    assert!(cheap_up.contains("50 µUSDC per 60 s (3000 µUSDC per hour)"));
    assert!(cheap_up.contains("no standbys"));
    let dear_up = row("DearUp");
    assert!(dear_up.contains("data-status=\"live\""), "{dear_up}");
    assert!(dear_up.contains("1000 µUSDC per 7 s (514285 + 5/7 µUSDC per hour)"));
    assert!(dear_up.contains("<td>40 µUSDC</td>"));
    let cheap_down = row("CheapDown");
    assert!(cheap_down.contains("data-status=\"down\""), "{cheap_down}");
    assert!(cheap_down.contains("1 µUSDC per 3600 s (1 µUSDC per hour)"));
}

#[tokio::test]
async fn compute_filters_by_query_and_marks_hidden_providers_without_a_host() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let (open, hidden) = (Keys::generate(), Keys::generate());
    store(
        relay,
        &event(&open, 10_432, "{}", &[&["L", "toon.network"]]),
    )
    .await;
    store(
        relay,
        &event(
            &hidden,
            10_432,
            r#"{"host":"secret.example.net"}"#,
            &[&["L", "toon.network"], &["l", "hidden:true"]],
        ),
    )
    .await;
    for (keys, d, labels) in [
        (&open, "Open", [&["l", "arch:arm64"][..], &["t", "docker"]]),
        (
            &hidden,
            "Hid",
            [&["l", "arch:x86_64"][..], &["t", "docker"]],
        ),
    ] {
        let a = format!("10432:{}:", keys.public_key().to_hex());
        let (base, name, address) = (["L", "toon.network"], ["d", d], ["a", a.as_str()]);
        let mut tags: Vec<&[&str]> = vec![&base, &name, &address];
        tags.extend(labels);
        let content = format!(r#"{{"name":"{d}","host":"secret.example.net"}}"#);
        store(relay, &event(keys, 30_432, &content, &tags)).await;
    }
    let listed = |page: &Page| {
        (
            page.text.contains("data-listing=\"Open\""),
            page.text.contains("data-listing=\"Hid\""),
        )
    };
    assert_eq!(listed(&get(relay, "/compute").await), (true, true));
    assert_eq!(
        listed(&get(relay, "/compute?arch=arm64").await),
        (true, false)
    );
    assert_eq!(
        listed(&get(relay, "/compute?cap=docker").await),
        (true, true)
    );
    assert_eq!(
        listed(&get(relay, "/compute?hidden=true").await),
        (false, true)
    );
    assert_eq!(
        listed(&get(relay, "/compute?hidden=false").await),
        (true, false)
    );
    assert_eq!(
        listed(&get(relay, "/compute?arch=arm64&hidden=true").await),
        (false, false)
    );
    let page = get(relay, "/compute?arch=x86_64").await;
    assert!(page.text.contains("Anyone-only"));
    assert!(page.text.contains("value=\"x86_64\""));
    assert!(page.text.contains("method=\"get\""));
    assert!(!page.text.contains("secret.example.net"));
    assert!(
        !get(relay, "/compute")
            .await
            .text
            .contains("secret.example.net")
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
