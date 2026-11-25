//! `/social/trending` and the topic pages over events written through
//! `POST /write`, driven through the router with no socket: the windows,
//! the maker exclusion, weighting and reports for `?as=`, topics from `t`
//! tags and NIP-32 labels, and NIP-85 providers only when they exist.
#![cfg(all(feature = "ui-repo", feature = "ui-social"))]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{delivery, running, write};
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

fn npub(keys: &Keys) -> String {
    keys.public_key().to_bech32().expect("encodes")
}

/// The row of `coordinate` in a ranked page, from its `<li` to its `</li>`.
fn row<'a>(html: &'a str, coordinate: &str) -> Option<&'a str> {
    let start = html.find(&format!("data-repo=\"{coordinate}\""))?;
    let end = html[start..].find("</li>")?;
    Some(&html[start..start + end])
}

#[tokio::test]
async fn trending_counts_people_in_the_window_and_topics_list_repos() {
    let running = running().await;
    let relay = &running.relay;
    let (maya, kofi) = (Keys::generate(), Keys::generate());
    let (a, b, c) = (Keys::generate(), Keys::generate(), Keys::generate());
    let (viewer, friend, labeller, provider) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let now = Timestamp::now().as_secs();
    let day = 86_400;
    let tide = format!("30617:{}:tide", maya.public_key().to_hex());
    let sail = format!("30617:{}:sail", kofi.public_key().to_hex());

    store(
        relay,
        &event(
            &maya,
            30_617,
            now - 40 * day,
            "",
            &[
                &["d", "tide"],
                &["name", "tide"],
                &["description", "Tide charts"],
                &["t", "Rust"],
                &["maintainers", &kofi.public_key().to_hex()],
            ],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &kofi,
            30_617,
            now - 40 * day,
            "",
            &[&["d", "sail"], &["name", "sail"], &["t", "nostr"]],
        ),
    )
    .await;
    let star = |keys: &Keys, at: u64, repo: &str| {
        event(keys, 7, at, "+", &[&["a", repo], &["k", "30617"]])
    };
    // Today: a stars tide (twice: one person), and the maintainer stars it,
    // which does not count. This week: b comments, c forks. This month: a
    // talks about sail.
    store(relay, &star(&a, now - 100, &tide)).await;
    store(relay, &event(&a, 1, now - 90, "tide!", &[&["a", &tide]])).await;
    store(relay, &star(&kofi, now - 80, &tide)).await;
    store(
        relay,
        &event(
            &b,
            1111,
            now - 2 * day,
            "Nice",
            &[&["A", &tide], &["K", "30617"]],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &c,
            30_617,
            now - 3 * day,
            "",
            &[
                &["d", "tide-fork"],
                &["u", &format!("{tide}|https://example.com/tide.git")],
            ],
        ),
    )
    .await;
    store(
        relay,
        &event(&a, 1, now - 10 * day, "sail", &[&["a", &sail]]),
    )
    .await;

    let (status, html) = get(relay, "/social/trending?window=day").await;
    assert_eq!(status, StatusCode::OK);
    assert!(!html.contains("<script"));
    assert!(html.contains("href=\"/social/trending\""), "the header nav");
    assert!(html.contains("data-formula"), "the formula is printed");
    let today = row(&html, &tide).expect("tide trends today");
    assert!(today.contains("data-people=\"1\""), "{today}");
    assert!(today.contains("data-stars=\"2\""), "{today}");
    assert!(today.contains("data-forks=\"1\""), "{today}");
    assert!(today.contains("class=\"spark\""));
    assert!(today.contains("href=\"/social/t/rust\""), "{today}");
    assert!(row(&html, &sail).is_none());
    assert!(
        !html.contains("data-other-rankings"),
        "no NIP-85 assertion, no box"
    );

    let (_, html) = get(relay, "/social/trending").await;
    assert!(html.contains("seg on\" href=\"/social/trending?window=week\""));
    let week = row(&html, &tide).expect("tide trends this week");
    assert!(week.contains("data-people=\"3\""), "{week}");
    assert!(row(&html, &sail).is_none());

    let (_, html) = get(relay, "/social/trending?window=month").await;
    assert!(
        row(&html, &sail)
            .expect("sail this month")
            .contains("data-people=\"1\"")
    );

    // Weighted by a viewer who follows b and a friend; the friend reported
    // sail, so it is hidden.
    store(
        relay,
        &event(
            &viewer,
            3,
            now - 50,
            "",
            &[
                &["p", &b.public_key().to_hex()],
                &["p", &friend.public_key().to_hex()],
            ],
        ),
    )
    .await;
    store(
        relay,
        &event(&friend, 1984, now - 40, "", &[&["a", &sail, "spam"]]),
    )
    .await;
    let (_, html) = get(
        relay,
        &format!("/social/trending?window=month&as={}", npub(&viewer)),
    )
    .await;
    assert!(html.contains("data-weighted"));
    assert!(html.contains("1 repo is hidden"), "{html}");
    assert!(row(&html, &sail).is_none(), "reported by a follow");
    let weighted = row(&html, &tide).expect("tide");
    assert!(weighted.contains("data-score=\"5\""), "{weighted}");

    // Topics: tide is rust by its owner; sail by a label.
    store(
        relay,
        &event(
            &labeller,
            1985,
            now - 30,
            "",
            &[&["L", "#t"], &["l", "rust", "#t"], &["a", &sail]],
        ),
    )
    .await;
    let (status, html) = get(relay, "/social/t/rust").await;
    assert_eq!(status, StatusCode::OK);
    let tagged = row(&html, &tide).expect("tide");
    assert!(tagged.contains("Tagged by its owner"));
    assert!(tagged.contains("data-forks=\"1\""), "{tagged}");
    let labelled = row(&html, &sail).expect("sail by label");
    assert!(labelled.contains("data-labeller"));
    assert!(labelled.contains(&npub(&labeller)));
    let (_, index) = get(relay, "/social/t").await;
    assert!(index.contains("href=\"/social/t/rust\">rust<span class=\"count\">2</span>"));
    let (_, html) = get(relay, "/social/trending?window=week&topic=nostr").await;
    assert!(row(&html, &tide).is_none());
    let (_, feed) = get(relay, "/social").await;
    assert!(feed.contains("data-topics"));
    assert!(feed.contains("data-trending"));
    let (_, page) = get(relay, &format!("/social/r/{}/tide", npub(&maya))).await;
    assert!(page.contains("class=\"chip chip-topic\" href=\"/social/t/rust\""));

    // A NIP-85 provider's assertion brings the Other rankings box.
    store(
        relay,
        &event(
            &provider,
            30_384,
            now - 20,
            "",
            &[&["d", &tide], &["rank", "77"]],
        ),
    )
    .await;
    let (_, html) = get(relay, "/social/trending").await;
    assert!(html.contains(&format!("data-provider=\"{}\"", npub(&provider))));
    let (_, html) = get(relay, &format!("/social/trending?by={}", npub(&provider))).await;
    assert!(
        row(&html, &tide)
            .expect("ranked")
            .contains("data-rank=\"77\"")
    );

    assert_eq!(get(relay, "/social/t/%20").await.0, StatusCode::NOT_FOUND);
}

/// One repo, two forks (one bare coordinate, one `|<git url>`) and one copy
/// that only shares the earliest unique commit: every page that counts forks
/// shows 2 (#367).
#[tokio::test]
async fn every_page_shows_the_same_fork_count() {
    let running = running().await;
    let relay = &running.relay;
    let (maya, bare, url, copier) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let now = Timestamp::now().as_secs();
    let tide = format!("30617:{}:tide", maya.public_key().to_hex());
    let commit = "a".repeat(40);
    store(
        relay,
        &event(
            &maya,
            30_617,
            now - 100,
            "",
            &[
                &["d", "tide"],
                &["name", "tide"],
                &["t", "rust"],
                &["r", &commit, "euc"],
            ],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &bare,
            30_617,
            now - 90,
            "",
            &[&["d", "tide"], &["u", &tide], &["r", &commit, "euc"]],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &url,
            30_617,
            now - 80,
            "",
            &[
                &["d", "tide"],
                &["u", &format!("{tide}|https://example.com/tide.git")],
            ],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &copier,
            30_617,
            now - 70,
            "",
            &[&["d", "tide-copy"], &["r", &commit, "euc"]],
        ),
    )
    .await;

    let base = format!("/social/r/{}/tide", npub(&maya));
    for path in [base.clone(), format!("{base}/forks")] {
        let (status, html) = get(relay, &path).await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            html.matches("Forks<span class=\"count\">2</span>").count() >= 1,
            "{path}"
        );
        assert!(
            !html.contains("Forks<span class=\"count\">3</span>"),
            "{path}"
        );
    }
    let (_, forks) = get(relay, &format!("{base}/forks")).await;
    assert!(
        forks.contains(">same first commit<"),
        "the copy stays listed"
    );
    assert!(forks.contains("2 forks"), "the glance: {forks}");
    for path in ["/social/trending?window=week", "/social/t/rust"] {
        let (_, html) = get(relay, path).await;
        let found = row(&html, &tide).unwrap_or_else(|| panic!("{path}"));
        assert!(found.contains("data-forks=\"2\""), "{path}: {found}");
    }
}
