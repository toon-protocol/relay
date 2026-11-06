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
use nostr::types::Timestamp;
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

    // A live Provider's page says when its current Liveness was signed.
    let shown = get(relay, &format!("/compute/p/{}", npub_of(&up))).await;
    assert!(shown.text.contains("data-live-since=\""), "{}", shown.text);
    assert!(shown.text.contains("Liveness signed"), "{}", shown.text);
    let shown = get(relay, &format!("/compute/p/{}", npub_of(&down))).await;
    assert!(!shown.text.contains("data-live-since"), "{}", shown.text);
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
            &[&["L", "toon.network"]],
        ),
    )
    .await;
    for (keys, d, labels) in [
        (
            &open,
            "Open",
            vec![
                &["l", "arch:arm64"][..],
                &["l", "isolation:vm"],
                &["l", "gpu:a100"],
                &["t", "docker"],
            ],
        ),
        (
            &hidden,
            "Hid",
            vec![
                &["l", "arch:x86_64"][..],
                &["l", "isolation:container"],
                &["l", "hidden:true"],
                &["t", "docker"],
            ],
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
        listed(&get(relay, "/compute?isolation=container").await),
        (false, true)
    );
    assert_eq!(
        listed(&get(relay, "/compute?gpu=a100").await),
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
    assert_eq!(
        listed(
            &get(
                relay,
                "/compute?arch=arm64&isolation=vm&gpu=a100&cap=docker&hidden=false"
            )
            .await
        ),
        (true, false)
    );
    // Each row shows what its filters match: isolation, GPU, capabilities.
    let page = get(relay, "/compute").await;
    let open_row = {
        let rest = &page.text[page.text.find("data-listing=\"Open\"").expect("Open")..];
        rest[..rest.find("</tr>").expect("a row end")].to_owned()
    };
    for shown in ["<td>vm</td>", "gpu a100", "<td>docker</td>"] {
        assert!(open_row.contains(shown), "{shown}: {open_row}");
    }
    let one = get(relay, &format!("/compute/{}/Open", npub_of(&open))).await;
    for shown in ["Isolation", "vm", "Capabilities", "docker"] {
        assert!(one.text.contains(shown), "{shown}: {}", one.text);
    }

    // A Listing labelled hidden makes its Provider Anyone-only on every page.
    let page = get(relay, &format!("/compute/p/{}", npub_of(&hidden))).await;
    assert!(page.text.contains("Anyone-only"), "{}", page.text);
    assert!(!page.text.contains("secret.example.net"), "{}", page.text);

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

#[tokio::test]
async fn a_provider_and_a_listing_have_pages_linked_with_social() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let provider = Keys::generate();
    let npub = nostr::nips::nip19::ToBech32::to_bech32(&provider.public_key()).expect("npub");
    store(
        relay,
        &event(
            &provider,
            10_432,
            r#"{"name":"Tidal","isolation":"vm","settlement":[{"chain":"base","tokens":["USDC"]}],
                "relay_set":["wss://relay.example"]}"#,
            &[&["L", "toon.network"]],
        ),
    )
    .await;
    listing(
        relay,
        &provider,
        "small",
        r#"{"name":"Small","resources":{"vcpu":2},"price":5,"lease_interval_s":60}"#,
    )
    .await;

    let page = get(relay, &format!("/compute/p/{npub}")).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.text);
    for shown in [
        "Tidal",
        "vm",
        "base: USDC",
        "wss://relay.example",
        "vcpu 2",
        "5 µUSDC per 60 s",
        &format!("href=\"/compute/{npub}/small\""),
        &format!("href=\"/social/p/{npub}\""),
    ] {
        assert!(page.text.contains(shown), "{shown}: {}", page.text);
    }

    let one = get(relay, &format!("/compute/{npub}/small")).await;
    assert_eq!(one.status, StatusCode::OK, "{}", one.text);
    assert!(one.text.contains("vcpu 2"));
    assert!(one.text.contains(&format!("href=\"/compute/p/{npub}\"")));

    let index = get(relay, "/compute").await;
    assert!(index.text.contains(&format!("href=\"/compute/p/{npub}\"")));
    assert!(
        index
            .text
            .contains(&format!("href=\"/compute/{npub}/small\""))
    );

    let coordinate = nostr::nips::nip01::Coordinate::new(Kind::from(30_432), provider.public_key())
        .identifier("small");
    let naddr = nostr::nips::nip19::ToBech32::to_bech32(&nostr::nips::nip19::Nip19Coordinate::new(
        coordinate,
        Vec::<nostr::types::RelayUrl>::new(),
    ))
    .expect("naddr");
    let response = running
        .relay
        .read_router()
        .oneshot(
            Request::get(format!("/compute/{naddr}"))
                .body(Body::empty())
                .expect("a request"),
        )
        .await
        .expect("infallible");
    assert!(response.status().is_redirection());
    assert_eq!(
        response.headers()["location"],
        format!("/compute/{npub}/small").as_str()
    );

    for path in [
        format!("/compute/{npub}/none"),
        format!(
            "/compute/p/{}",
            nostr::nips::nip19::ToBech32::to_bech32(&Keys::generate().public_key()).unwrap()
        ),
    ] {
        let missing = get(relay, &path).await;
        assert_eq!(missing.status, StatusCode::NOT_FOUND, "{path}");
        assert!(missing.text.contains("Not found"), "{path}");
    }
}

#[tokio::test]
async fn a_social_profile_shows_the_compute_badge_only_for_a_provider_while_compute_is_on() {
    let provider = Keys::generate();
    let plain = Keys::generate();
    let npub = |k: &Keys| nostr::nips::nip19::ToBech32::to_bech32(&k.public_key()).expect("npub");
    for apps in [None, Some("repo,social")] {
        let running = running_with(|name| {
            (name == "TOON_UI_APPS")
                .then(|| apps.map(str::to_owned))
                .flatten()
        })
        .await;
        let relay = &running.relay;
        store(
            relay,
            &event(&provider, 10_432, "{}", &[&["L", "toon.network"]]),
        )
        .await;
        store(relay, &event(&plain, 0, "{}", &[])).await;
        let with = get(relay, &format!("/social/p/{}", npub(&provider))).await;
        let without = get(relay, &format!("/social/p/{}", npub(&plain))).await;
        assert_eq!(with.status, StatusCode::OK);
        assert_eq!(
            with.text
                .contains(&format!("href=\"/compute/p/{}\"", npub(&provider))),
            apps.is_none(),
            "{apps:?}"
        );
        assert!(!without.text.contains("data-compute-provider"));
    }
}

#[tokio::test]
async fn an_empty_d_has_a_page_and_only_listing_naddrs_and_labelled_profiles_count() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let to_npub =
        |k: &Keys| nostr::nips::nip19::ToBech32::to_bech32(&k.public_key()).expect("npub");
    let provider = Keys::generate();
    let npub = to_npub(&provider);
    store(
        relay,
        &event(&provider, 10_432, "{}", &[&["L", "toon.network"]]),
    )
    .await;
    listing(relay, &provider, "", r#"{"name":"Unnamed"}"#).await;

    // A Listing with an empty `d` still has a page its link reaches.
    let page = get(relay, &format!("/compute/p/{npub}")).await;
    assert!(page.text.contains(&format!("href=\"/compute/{npub}/\"")));
    let one = get(relay, &format!("/compute/{npub}/")).await;
    assert_eq!(one.status, StatusCode::OK, "{}", one.text);
    assert!(one.text.contains("Unnamed"));

    // An `naddr` of another kind is not a Listing's.
    let coordinate = nostr::nips::nip01::Coordinate::new(Kind::from(30_023), provider.public_key())
        .identifier("small");
    let naddr = nostr::nips::nip19::ToBech32::to_bech32(&nostr::nips::nip19::Nip19Coordinate::new(
        coordinate,
        Vec::<nostr::types::RelayUrl>::new(),
    ))
    .expect("naddr");
    assert_eq!(
        get(relay, &format!("/compute/{naddr}")).await.status,
        StatusCode::NOT_FOUND
    );

    // A 10432 without TOON Compute's label is no Provider: no page, no badge.
    let unlabelled = Keys::generate();
    store(relay, &event(&unlabelled, 10_432, "{}", &[])).await;
    let page = get(relay, &format!("/compute/p/{}", to_npub(&unlabelled))).await;
    assert_eq!(page.status, StatusCode::NOT_FOUND);
    let social = get(relay, &format!("/social/p/{}", to_npub(&unlabelled))).await;
    assert_eq!(social.status, StatusCode::OK);
    assert!(!social.text.contains("data-compute-provider"));
}

fn npub_of(keys: &Keys) -> String {
    nostr::nips::nip19::ToBech32::to_bech32(&keys.public_key()).expect("npub")
}

fn in_an_hour() -> String {
    (nostr::types::Timestamp::now().as_secs() + 3_600).to_string()
}

/// A Deployment of `tenant`'s `site` repository, `env`, on `primary` with
/// `standby`, whose workload id is `workload`.
fn deployment_event(
    tenant: &Keys,
    env: &str,
    primary: &Keys,
    standby: &Keys,
    workload: &str,
    extra: &[&[&str]],
) -> Event {
    let (tenant_hex, primary_hex, standby_hex) = (
        tenant.public_key().to_hex(),
        primary.public_key().to_hex(),
        standby.public_key().to_hex(),
    );
    let repo = format!("30617:{tenant_hex}:site");
    let listing = format!("30432:{primary_hex}:small");
    let expiration = in_an_hour();
    let commit = "1".repeat(40);
    let image = "cd".repeat(32);
    let mut tags: Vec<Vec<&str>> = vec![
        vec!["d", env],
        vec!["a", &repo],
        vec!["c", &commit],
        vec!["a", &listing],
        vec!["p", &primary_hex, "primary"],
        vec!["p", &standby_hex, "standby"],
        vec!["x", workload, "workload"],
        vec!["x", &image, "image"],
        vec!["r", "https://app.example.com"],
        vec!["expiration", &expiration],
        vec!["L", "toon.network"],
    ];
    tags.extend(extra.iter().map(|tag| tag.to_vec()));
    let tags: Vec<&[&str]> = tags.iter().map(Vec::as_slice).collect();
    event(tenant, 30_437, "", &tags)
}

#[tokio::test]
async fn a_deployment_page_shows_each_state_and_ignores_keys_it_does_not_name() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let (tenant, primary, standby, stranger) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let (tenant_npub, primary_npub) = (npub_of(&tenant), npub_of(&primary));
    for provider in [&primary, &standby] {
        store(
            relay,
            &event(provider, 10_432, "{}", &[&["L", "toon.network"]]),
        )
        .await;
    }
    listing(relay, &primary, "small", r#"{"name":"Small"}"#).await;
    // The tenant owns `site`, so the Deployment is not a claim.
    store(relay, &event(&tenant, 30_617, "", &[&["d", "site"]])).await;

    let workload = "ab".repeat(32);
    store(
        relay,
        &deployment_event(&tenant, "production", &primary, &standby, &workload, &[]),
    )
    .await;
    let path = format!("/compute/d/{tenant_npub}/production");

    // No Liveness: the provider is down.
    let page = get(relay, &path).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.text);
    for shown in [
        "provider down",
        &format!("href=\"/repo/{tenant_npub}/site\""),
        &format!(
            "href=\"/repo/{tenant_npub}/site/commit/{}\"",
            "1".repeat(40)
        ),
        &format!("href=\"/compute/p/{primary_npub}\""),
        &format!("href=\"/compute/{primary_npub}/small\""),
        &format!("sha256:{}", "cd".repeat(32)),
        "href=\"https://app.example.com\"",
    ] {
        assert!(page.text.contains(shown), "{shown}: {}", page.text);
    }
    assert!(!page.text.contains("data-claim"), "{}", page.text);

    // Live.
    store(
        relay,
        &event(
            &primary,
            10_433,
            "{}",
            &[&["expiration", &in_an_hour()], &["L", "toon.network"]],
        ),
    )
    .await;
    let page = get(relay, &path).await;
    assert!(
        page.text.contains("running on a live provider until"),
        "{}",
        page.text
    );

    // Keys the Deployment does not name change nothing.
    store(relay, &event(&stranger, 4_433, "{}", &[&["x", &workload]])).await;
    store(
        relay,
        &event(
            &stranger,
            30_433,
            &format!(r#"{{"primary":"{}"}}"#, primary.public_key().to_hex()),
            &[&["d", &workload]],
        ),
    )
    .await;
    let page = get(relay, &path).await;
    assert!(
        page.text.contains("running on a live provider until"),
        "{}",
        page.text
    );

    // The provider lists it under Serving.
    let provider = get(relay, &format!("/compute/p/{primary_npub}")).await;
    assert!(provider.text.contains("Serving"), "{}", provider.text);
    assert!(
        provider.text.contains(&format!("href=\"{path}\"")),
        "{}",
        provider.text
    );
    assert!(provider.text.contains("running on a live provider"));

    // Taken over by the standby.
    store(
        relay,
        &event(
            &standby,
            30_433,
            &format!(r#"{{"primary":"{}"}}"#, primary.public_key().to_hex()),
            &[&["d", &workload]],
        ),
    )
    .await;
    let page = get(relay, &path).await;
    assert!(page.text.contains("taken over by"), "{}", page.text);
    assert!(
        page.text
            .contains(&format!("/compute/p/{}", npub_of(&standby)))
    );

    // Evicted by the primary, with its reason and message.
    store(
        relay,
        &event(
            &primary,
            4_433,
            r#"{"reason":"abuse","message":"mining <b>"}"#,
            &[&["x", &workload], &["L", "toon.network"]],
        ),
    )
    .await;
    let page = get(relay, &path).await;
    assert!(page.text.contains("evicted by"), "{}", page.text);
    assert!(page.text.contains("abuse"));
    assert!(page.text.contains("mining &lt;b&gt;"), "{}", page.text);
}

#[tokio::test]
async fn a_deployment_that_is_expired_or_misshapen_is_a_404_and_a_foreign_signer_is_a_claim() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let (tenant, owner, primary, standby) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let workload = "ab".repeat(32);

    // Not the repository's owner: labelled as the signer's claim.
    let mut claim = deployment_event(&tenant, "claim", &primary, &standby, &workload, &[]);
    let repo = format!("30617:{}:site", owner.public_key().to_hex());
    claim = rebuild(&tenant, &claim, |tag| {
        tag.starts_with("30617:").then(|| repo.clone())
    });
    store(relay, &claim).await;
    let page = get(relay, &format!("/compute/d/{}/claim", npub_of(&tenant))).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.text);
    assert!(page.text.contains("data-claim"), "{}", page.text);

    // The repository's owner is not a claim, even with no announcement here.
    store(
        relay,
        &deployment_event(&owner, "owned", &primary, &standby, &workload, &[]),
    )
    .await;
    let page = get(relay, &format!("/compute/d/{}/owned", npub_of(&owner))).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.text);
    assert!(!page.text.contains("data-claim"), "{}", page.text);

    // A maintainer named by the owner is not a claim.
    store(
        relay,
        &event(
            &owner,
            30_617,
            "",
            &[
                &["d", "site"],
                &["maintainers", &tenant.public_key().to_hex()],
            ],
        ),
    )
    .await;
    let page = get(relay, &format!("/compute/d/{}/claim", npub_of(&tenant))).await;
    assert!(!page.text.contains("data-claim"), "{}", page.text);

    // Past its expiration: no longer shown, and its URL is a 404.
    let soon = (nostr::types::Timestamp::now().as_secs() + 2).to_string();
    let expiring = deployment_event(&tenant, "expiring", &primary, &standby, &workload, &[]);
    let tags: Vec<Tag> = expiring
        .tags
        .iter()
        .map(|tag| {
            let mut parts = tag.as_slice().to_vec();
            if parts[0] == "expiration" {
                parts[1] = soon.clone();
            }
            Tag::parse(parts).expect("a tag")
        })
        .collect();
    let expiring = EventBuilder::new(Kind::from(30_437), "")
        .tags(tags)
        .finalize(&tenant)
        .expect("signs");
    store(relay, &expiring).await;
    let path = format!("/compute/d/{}/expiring", npub_of(&tenant));
    assert_eq!(get(relay, &path).await.status, StatusCode::OK);
    tokio::time::sleep(std::time::Duration::from_secs(3)).await;
    assert_eq!(get(relay, &path).await.status, StatusCode::NOT_FOUND);

    // An unmarked x breaks the shape.
    let broken = deployment_event(
        &tenant,
        "broken",
        &primary,
        &standby,
        &workload,
        &[&["x", &"ef".repeat(32)]],
    );
    store(relay, &broken).await;
    let page = get(relay, &format!("/compute/d/{}/broken", npub_of(&tenant))).await;
    assert_eq!(page.status, StatusCode::NOT_FOUND);

    // Nothing at all.
    let page = get(relay, &format!("/compute/d/{}/nothing", npub_of(&tenant))).await;
    assert_eq!(page.status, StatusCode::NOT_FOUND);

    // The naddr redirects to the page.
    let coordinate = nostr::nips::nip01::Coordinate::new(Kind::from(30_437), tenant.public_key())
        .identifier("claim");
    let naddr = nostr::nips::nip19::ToBech32::to_bech32(&nostr::nips::nip19::Nip19Coordinate::new(
        coordinate,
        Vec::<nostr::types::RelayUrl>::new(),
    ))
    .expect("naddr");
    let response = running
        .relay
        .read_router()
        .oneshot(
            Request::get(format!("/compute/{naddr}"))
                .body(Body::empty())
                .expect("a request"),
        )
        .await
        .expect("infallible");
    assert!(response.status().is_redirection());
    assert_eq!(
        response.headers()["location"],
        format!("/compute/d/{}/claim", npub_of(&tenant)).as_str()
    );
}

/// `source` signed again by `keys` with each `a` tag value `change` returns
/// `Some` for replaced.
fn rebuild(keys: &Keys, source: &Event, change: impl Fn(&str) -> Option<String>) -> Event {
    let tags: Vec<Tag> = source
        .tags
        .iter()
        .map(|tag| {
            let mut parts = tag.as_slice().to_vec();
            if parts[0] == "a"
                && let Some(new) = change(&parts[1])
            {
                parts[1] = new;
            }
            Tag::parse(parts).expect("a tag")
        })
        .collect();
    EventBuilder::new(Kind::from(30_437), "")
        .tags(tags)
        .finalize(keys)
        .expect("signs")
}

/// `event`, signed `ago` seconds before now.
fn event_ago(keys: &Keys, kind: u16, content: &str, tags: &[&[&str]], ago: u64) -> Event {
    EventBuilder::new(Kind::from(kind), content)
        .tags(
            tags.iter()
                .map(|tag| Tag::parse(tag.iter().copied()).expect("a tag")),
        )
        .custom_created_at(Timestamp::from_secs(Timestamp::now().as_secs() - ago))
        .finalize(keys)
        .expect("signs")
}

#[tokio::test]
async fn a_provider_page_logs_its_evictions_takeovers_and_deployments_newest_first() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let (tenant, primary, standby, other) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    for provider in [&primary, &standby, &other] {
        store(
            relay,
            &event(provider, 10_432, "{}", &[&["L", "toon.network"]]),
        )
        .await;
    }
    let (workload, elsewhere) = ("ab".repeat(32), "cd".repeat(32));
    store(
        relay,
        &deployment_event(&tenant, "production", &primary, &standby, &workload, &[]),
    )
    .await;
    let primary_json = format!(r#"{{"primary":"{}"}}"#, primary.public_key().to_hex());
    store(
        relay,
        &event_ago(&standby, 30_433, &primary_json, &[&["d", &workload]], 200),
    )
    .await;
    for (content, ago) in [
        (r#"{"reason":"made-up","message":"too loud"}"#, 100),
        (r#"{"reason":"abuse","message":"first"}"#, 300),
    ] {
        store(
            relay,
            &event_ago(&primary, 4_433, content, &[&["x", &"ef".repeat(32)]], ago),
        )
        .await;
    }
    // Another provider's Eviction Notice, Deployment and Takeover are not in
    // this provider's log.
    store(
        relay,
        &event(
            &other,
            4_433,
            r#"{"reason":"abuse","message":"elsewhere"}"#,
            &[&["x", &"12".repeat(32)]],
        ),
    )
    .await;
    store(
        relay,
        &deployment_event(&tenant, "staging", &other, &standby, &elsewhere, &[]),
    )
    .await;
    let other_json = format!(r#"{{"primary":"{}"}}"#, other.public_key().to_hex());
    store(
        relay,
        &event_ago(&standby, 30_433, &other_json, &[&["d", &elsewhere]], 50),
    )
    .await;

    let page = get(relay, &format!("/compute/p/{}", npub_of(&primary))).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.text);
    let log = &page.text[page.text.find("data-activity>").expect("activity log")..];
    for shown in [
        "made-up",
        "too loud",
        "took over from",
        &format!("/compute/p/{}", npub_of(&standby)),
        &format!("/compute/d/{}/production", npub_of(&tenant)),
    ] {
        assert!(log.contains(shown), "{shown}: {log}");
    }
    for hidden in ["elsewhere", "staging"] {
        assert!(!log.contains(hidden), "{hidden}: {log}");
    }
    let rows: Vec<(&str, u64)> = log
        .split("data-activity-kind=\"")
        .skip(1)
        .map(|row| {
            let kind = &row[..row.find('"').expect("a kind")];
            let at = &row[row.find("data-at=\"").expect("a time") + 9..];
            (
                kind,
                at[..at.find('"').expect("a time")]
                    .parse()
                    .expect("a number"),
            )
        })
        .collect();
    let kinds: Vec<&str> = rows.iter().map(|(kind, _)| *kind).collect();
    assert_eq!(
        kinds,
        ["deployment", "eviction", "takeover", "eviction"],
        "{log}"
    );
    assert!(rows.windows(2).all(|w| w[0].1 > w[1].1), "{rows:?}");

    // The other provider's own page logs its own events alone.
    let page = get(relay, &format!("/compute/p/{}", npub_of(&other))).await;
    let log = &page.text[page.text.find("data-activity>").expect("activity log")..];
    assert!(
        log.contains("elsewhere") && log.contains("staging"),
        "{log}"
    );
    assert!(
        !log.contains("too loud") && !log.contains("production"),
        "{log}"
    );
}

#[tokio::test]
async fn the_environments_tab_lists_only_the_owners_and_maintainers_deployments() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let (owner, maintainer, stranger, primary, standby) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let workload = "ab".repeat(32);
    let repo = format!("30617:{}:site", owner.public_key().to_hex());
    let of_owner = |keys: &Keys, env: &str| {
        let made = deployment_event(keys, env, &primary, &standby, &workload, &[]);
        rebuild(keys, &made, |tag| {
            tag.starts_with("30617:").then(|| repo.clone())
        })
    };
    let announce = |maintainers: &[&Keys], at: u64| {
        let keys: Vec<String> = maintainers
            .iter()
            .map(|m| m.public_key().to_hex())
            .collect();
        let mut tags: Vec<Vec<String>> = vec![vec!["d".into(), "site".into()]];
        if !keys.is_empty() {
            let mut tag = vec!["maintainers".to_owned()];
            tag.extend(keys);
            tags.push(tag);
        }
        EventBuilder::new(Kind::from(30_617), "")
            .tags(tags.into_iter().map(|t| Tag::parse(t).expect("a tag")))
            .custom_created_at(nostr::types::Timestamp::from_secs(at))
            .finalize(&owner)
            .expect("signs")
    };
    let now = nostr::types::Timestamp::now().as_secs();
    store(relay, &announce(&[&maintainer], now - 100)).await;
    for (keys, env) in [
        (&owner, "prod"),
        (&maintainer, "staging"),
        (&stranger, "fake"),
    ] {
        store(relay, &of_owner(keys, env)).await;
    }

    let tab = format!("/repo/{}/site/environments", npub_of(&owner));
    let page = get(relay, &tab).await;
    assert_eq!(page.status, StatusCode::OK, "{}", page.text);
    assert!(page.text.contains("data-environment=\"prod\""));
    assert!(page.text.contains("data-environment=\"staging\""));
    assert!(!page.text.contains("data-environment=\"fake\""));
    assert!(page.text.contains("data-visit"));
    assert!(
        page.text.contains("running on a live provider") || page.text.contains("provider down")
    );
    // The stranger's Deployment still has its page, labelled as a claim.
    let own = get(relay, &format!("/compute/d/{}/fake", npub_of(&stranger))).await;
    assert_eq!(own.status, StatusCode::OK);
    assert!(own.text.contains("data-claim"));
    // Other tabs link to it.
    let home = get(relay, &format!("/repo/{}/site", npub_of(&owner))).await;
    assert!(home.text.contains("/environments"));

    // The same state as the Deployment page.
    let deployment = get(
        relay,
        &format!("/compute/d/{}/staging", npub_of(&maintainer)),
    )
    .await;
    let state = |text: &str| {
        text.split("data-state=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .map(str::to_owned)
    };
    let row = page
        .text
        .split("data-environment=\"staging\"")
        .nth(1)
        .and_then(|rest| rest.split("</tr>").next())
        .expect("the staging row");
    assert!(state(row).is_some(), "{row}");
    assert_eq!(state(row), state(&deployment.text));
    // Dropping the maintainer from the owner's 30617 drops their Deployment.
    store(relay, &announce(&[], now)).await;
    let page = get(relay, &tab).await;
    assert!(page.text.contains("data-environment=\"prod\""));
    assert!(!page.text.contains("data-environment=\"staging\""));
}

#[tokio::test]
async fn with_compute_off_the_environments_tab_is_absent() {
    let running =
        running_with(|name| (name == "TOON_UI_APPS").then(|| "repo,social".to_owned())).await;
    let relay = &running.relay;
    let owner = Keys::generate();
    store(relay, &event(&owner, 30_617, "", &[&["d", "site"]])).await;
    let home = get(relay, &format!("/repo/{}/site", npub_of(&owner))).await;
    assert_eq!(home.status, StatusCode::OK);
    assert!(!home.text.contains("Environments"), "{}", home.text);
    let tab = get(
        relay,
        &format!("/repo/{}/site/environments", npub_of(&owner)),
    )
    .await;
    assert_eq!(tab.status, StatusCode::NOT_FOUND);
}

/// The `content` of the `<meta>` named `name` in `html`.
fn meta(html: &str, name: &str) -> Option<String> {
    let needle = format!("=\"{name}\" content=\"");
    let start = html.find(&needle)? + needle.len();
    let rest = &html[start..];
    Some(rest[..rest.find('"')?].to_owned())
}

#[tokio::test]
async fn provider_listing_and_deployment_pages_name_their_cards_which_are_pngs_with_etags() {
    const ORIGIN: &str = "https://relay.test";
    let running =
        running_with(|name| (name == "TOON_UI_PUBLIC_URL").then(|| format!("{ORIGIN}/"))).await;
    let relay = &running.relay;
    let (tenant, primary, standby) = (Keys::generate(), Keys::generate(), Keys::generate());
    let (tenant_npub, primary_npub) = (npub_of(&tenant), npub_of(&primary));
    for provider in [&primary, &standby] {
        store(
            relay,
            &event(
                provider,
                10_432,
                r#"{"name":"Tidal"}"#,
                &[&["L", "toon.network"]],
            ),
        )
        .await;
    }
    listing(
        relay,
        &primary,
        "small",
        r#"{"name":"Small","resources":{"vcpu":2},"price":5,"lease_interval_s":60}"#,
    )
    .await;
    store(relay, &event(&tenant, 30_617, "", &[&["d", "site"]])).await;
    store(
        relay,
        &deployment_event(
            &tenant,
            "production",
            &primary,
            &standby,
            &"ab".repeat(32),
            &[],
        ),
    )
    .await;

    let pages = [
        (
            format!("/compute/p/{primary_npub}"),
            format!("/compute/og/p/{primary_npub}.png"),
        ),
        (
            format!("/compute/{primary_npub}/small"),
            format!("/compute/og/l/{primary_npub}/small.png"),
        ),
        (
            format!("/compute/d/{tenant_npub}/production"),
            format!("/compute/og/d/{tenant_npub}/production.png"),
        ),
    ];
    for (page, card) in pages {
        let shown = get(relay, &page).await;
        assert_eq!(shown.status, StatusCode::OK, "{page}");
        // The page's policy is the directory's own.
        assert_eq!(shown.policy.as_deref(), Some(POLICY), "{page}");
        for tag in ["og:image", "twitter:image"] {
            let image = meta(&shown.text, tag).unwrap_or_else(|| panic!("{page}: {tag}"));
            assert_eq!(image, format!("{ORIGIN}{card}"), "{page}");
        }
        assert_eq!(
            meta(&shown.text, "twitter:card").as_deref(),
            Some("summary_large_image")
        );

        let response = relay
            .read_router()
            .oneshot(Request::get(&card).body(Body::empty()).expect("a request"))
            .await
            .expect("infallible");
        assert_eq!(response.status(), StatusCode::OK, "{card}");
        assert_eq!(response.headers()["content-type"], "image/png");
        let etag = response.headers()["etag"].clone();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n", "{card}");
        let word = |at: usize| u32::from_be_bytes(bytes[at..at + 4].try_into().expect("four"));
        assert_eq!((word(16), word(20)), (1200, 630), "{card}");

        let again = relay
            .read_router()
            .oneshot(
                Request::get(&card)
                    .header("if-none-match", etag)
                    .body(Body::empty())
                    .expect("a request"),
            )
            .await
            .expect("infallible");
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED, "{card}");
    }

    for missing in [
        format!("/compute/og/p/{}.png", npub_of(&Keys::generate())),
        format!("/compute/og/l/{primary_npub}/nope.png"),
        format!("/compute/og/d/{tenant_npub}/nope.png"),
    ] {
        assert_eq!(
            get(relay, &missing).await.status,
            StatusCode::NOT_FOUND,
            "{missing}"
        );
    }
}

#[tokio::test]
async fn a_listing_page_works_out_a_request_for_an_agent() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let provider = Keys::generate();
    let npub = npub_of(&provider);
    store(
        relay,
        &event(
            &provider,
            10_432,
            r#"{"name":"Tidal"}"#,
            &[&["L", "toon.network"]],
        ),
    )
    .await;
    listing(
        relay,
        &provider,
        "slow",
        r#"{"name":"Slow","price":900,"lease_interval_s":600,"standby_price":40}"#,
    )
    .await;
    listing(
        relay,
        &provider,
        "quick",
        r#"{"name":"Quick","price":3,"lease_interval_s":7}"#,
    )
    .await;
    listing(relay, &provider, "bare", r#"{"name":"Bare"}"#).await;
    let naddr = nostr::nips::nip19::ToBech32::to_bech32(&nostr::nips::nip19::Nip19Coordinate::new(
        nostr::nips::nip01::Coordinate::new(Kind::from(30_432), provider.public_key())
            .identifier("slow"),
        [],
    ))
    .expect("naddr");

    let base = format!("/compute/{npub}/slow");
    let empty = get(relay, &base).await;
    assert_eq!(empty.status, StatusCode::OK);
    assert!(empty.text.contains("Use with an agent"));
    assert!(!empty.text.contains("data-agent-request"));
    assert!(!empty.text.contains("data-agent-command"));
    assert!(
        empty
            .text
            .contains("a spawn would fail until the Provider is live")
    );

    let two = get(relay, &format!("{base}?hours=2&image=nginx")).await;
    assert_eq!(two.status, StatusCode::OK);
    assert_eq!(two.policy.as_deref(), Some(POLICY));
    assert!(
        two.text.contains("12 Lease Intervals, 10800 µUSDC"),
        "{}",
        two.text
    );
    assert!(two.text.contains("480 µUSDC for one Warm Standby"));
    assert!(
        two.text
            .contains(&format!("Run `nginx` on Listing `{naddr}` (Tidal / slow)"))
    );
    assert!(
        two.text
            .contains("for 2 hours (12 × 10 min Lease Intervals), port &lt;port&gt;")
    );
    assert!(two.text.contains("at most 10800 µUSDC"));
    assert!(
        two.text.contains(&format!(
            "<pre data-agent-command>toon compute spawn {naddr} --intervals 12 --max 10800 \
             --image nginx --port &lt;port&gt; --yes</pre>"
        )),
        "{}",
        two.text
    );

    let five = get(relay, &format!("{base}?intervals=5")).await;
    assert!(five.text.contains("5 × 10 min Lease Intervals"));
    assert!(five.text.contains("4500 µUSDC"));
    assert!(five.text.contains("&lt;image&gt;"));
    assert!(five.text.contains("--intervals 5 --max 4500 --image &lt;repo&gt;@sha256:&lt;digest&gt; --port &lt;port&gt; --yes</pre>"));
    let typed = get(
        relay,
        &format!("{base}?intervals=3&image=a%22%3Cb%3E&port=80%26x"),
    )
    .await;
    assert!(
        typed.text.contains(
            "--intervals 3 --max 2700 --image a&quot;&lt;b&gt; --port 80&amp;x --yes</pre>"
        ),
        "{}",
        typed.text
    );
    let quick = get(relay, &format!("/compute/{npub}/quick?hours=2")).await;
    assert!(
        quick.text.contains("1029 Lease Intervals, 3087 µUSDC"),
        "{}",
        quick.text
    );
    assert!(!quick.text.contains("Warm Standby"));
    let month = get(relay, &format!("/compute/{npub}/quick?hours=720")).await;
    assert!(month.text.contains("data-agent-request"), "{}", month.text);
    let one = get(relay, &format!("{base}?intervals=1")).await;
    assert!(one.text.contains("for 1 × 10 min Lease Interval, port"));

    for query in [
        "hours=abc",
        "hours=0",
        "hours=-1",
        "hours=721",
        "hours=1&intervals=2",
        "intervals=0",
        "intervals=99999999999999999999",
        "hours=18446744073709551615",
        "intervals=4321",
    ] {
        let page = get(relay, &format!("{base}?{query}")).await;
        assert_eq!(page.status, StatusCode::OK, "{query}");
        assert!(page.text.contains("data-agent-note"), "{query}");
        assert!(!page.text.contains("data-agent-request"), "{query}");
        assert!(!page.text.contains("data-agent-command"), "{query}");
        assert!(!page.text.contains("value=\"1"), "{query}");
        assert!(!page.text.contains("value=\"abc"), "{query}");
    }
    let bare = get(relay, &format!("/compute/{npub}/bare?hours=1")).await;
    assert_eq!(bare.status, StatusCode::OK);
    assert!(bare.text.contains("states no price or interval"));
    assert!(!bare.text.contains("data-agent-request"));

    let script = get(relay, &format!("{base}?hours=1&image=%3Cscript%3E")).await;
    assert!(script.text.contains("&lt;script&gt;"));
    assert!(!script.text.contains("<script>"));
}
