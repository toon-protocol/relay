//! The builder profile, `/social/p/<npub>`, over events written through
//! `POST /write`, driven through the router with no socket: the identity
//! column, the tabs, the activity graph, and the links to it from the rest
//! of `/social` and from `/repo`.
#![cfg(all(feature = "ui-repo", feature = "ui-social"))]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{delivery, running_with, write};
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
    assert_eq!(
        write(relay, delivery(event)).await.0,
        200,
        "kind {}",
        event.kind
    );
}

fn npub(keys: &Keys) -> String {
    keys.public_key().to_bech32().expect("encodes")
}

#[tokio::test]
async fn a_profile_shows_who_a_key_says_it_is_and_what_it_built() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let (maya, kofi, dan, bot) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let (maya_hex, kofi_hex) = (maya.public_key().to_hex(), kofi.public_key().to_hex());
    let now = Timestamp::now().as_secs();
    let tidepool = format!("30617:{maya_hex}:tidepool");
    let nmea = format!("30617:{maya_hex}:nmea-lite");
    let gitsail = format!("30617:{kofi_hex}:gitsail");
    let badge = format!("30009:{kofi_hex}:first-merge");

    store(
        relay,
        &event(
            &maya,
            0,
            now - 500,
            r#"{"name":"Maya Okafor","about":"Builds <b>tide</b> tools","website":"https://tide.dev",
               "banner":"https://cdn.test/banner.jpg","nip05":"maya@tide.dev"}"#,
            &[],
        ),
    )
    .await;
    store(
        relay,
        &event(&bot, 0, now - 500, r#"{"name":"shipbot","bot":true}"#, &[]),
    )
    .await;
    store(
        relay,
        &event(&kofi, 0, now - 500, r#"{"name":"kofi"}"#, &[]),
    )
    .await;
    for (d, at) in [("tidepool", now - 400), ("nmea-lite", now - 390)] {
        store(
            relay,
            &event(
                &maya,
                30_617,
                at,
                "",
                &[&["d", d], &["name", d], &["description", "A repo"]],
            ),
        )
        .await;
    }
    store(
        relay,
        &event(
            &kofi,
            30_617,
            now - 380,
            "",
            &[
                &["d", "gitsail"],
                &["name", "gitsail"],
                &["maintainers", &maya_hex],
            ],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &maya,
            30_315,
            now - 300,
            "Shipping wind overlays for",
            &[
                &["d", "general"],
                &["a", &tidepool],
                &["expiration", &(now + 3_600).to_string()],
            ],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &maya,
            10_011,
            now - 300,
            "",
            &[&["i", "github:mayaokafor", "9721ce4ee4fceb91c9711ca2a6c9a5ab"]],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &maya,
            10_617,
            now - 300,
            "",
            &[&["a", &nmea], &["a", &gitsail]],
        ),
    )
    .await;
    store(relay, &event(&dan, 3, now - 300, "", &[&["p", &maya_hex]])).await;
    store(relay, &event(&kofi, 3, now - 300, "", &[&["p", &maya_hex]])).await;
    store(relay, &event(&maya, 3, now - 300, "", &[&["p", &kofi_hex]])).await;
    store(
        relay,
        &event(&maya, 10_018, now - 300, "", &[&["a", &gitsail]]),
    )
    .await;
    store(
        relay,
        &event(
            &maya,
            7,
            now - 290,
            "+",
            &[&["a", &gitsail], &["k", "30617"]],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &kofi,
            30_009,
            now - 280,
            "",
            &[&["d", "first-merge"], &["name", "First merged PR"]],
        ),
    )
    .await;
    let award = event(
        &kofi,
        8,
        now - 270,
        "",
        &[&["a", &badge], &["p", &maya_hex]],
    );
    store(relay, &award).await;
    store(
        relay,
        &event(
            &maya,
            10_008,
            now - 260,
            "",
            &[&["a", &badge], &["e", &award.id.to_hex()]],
        ),
    )
    .await;
    // Activity: an issue, a merge and a comment (two repos), a note; and a
    // repo state, which is not counted.
    let issue = event(
        &maya,
        1621,
        now - 200,
        "",
        &[&["a", &gitsail], &["subject", "Cache refs"]],
    );
    store(relay, &issue).await;
    store(
        relay,
        &event(
            &maya,
            1631,
            now - 190,
            "",
            &[&["e", &issue.id.to_hex(), "", "root"], &["a", &gitsail]],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &maya,
            1111,
            now - 180,
            "Wind is next",
            &[&["A", &tidepool], &["K", "30617"]],
        ),
    )
    .await;
    let note = event(
        &maya,
        1,
        now - 170,
        "Shipped tidepool today",
        &[&["a", &tidepool]],
    );
    store(relay, &note).await;
    store(
        relay,
        &event(
            &maya,
            30_618,
            now - 160,
            "",
            &[&["d", "tidepool"], &["refs/heads/main", &"a".repeat(40)]],
        ),
    )
    .await;

    let base = format!("/social/p/{}", npub(&maya));
    let (status, html) = get(relay, &base).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!html.contains("<script"));
    assert!(
        html.contains("<h1 class=\"profile-name\">Maya Okafor</h1>"),
        "{html}"
    );
    assert!(html.contains("Builds &lt;b&gt;tide&lt;/b&gt; tools"));
    assert!(html.contains("<img class=\"banner\" src=\"https://cdn.test/banner.jpg\""));
    assert!(html.contains("maya@tide.dev</span><span class=\"chip\""));
    assert!(html.contains(">unverified</span>"));
    assert!(html.contains("data-status=\"general\""));
    assert!(html.contains("Shipping wind overlays for"));
    assert!(
        html.contains(
            "href=\"https://gist.github.com/mayaokafor/9721ce4ee4fceb91c9711ca2a6c9a5ab\""
        )
    );
    assert!(html.contains("data-followers=\"2\""), "dan and kofi");
    assert!(html.contains("data-following=\"1\""));
    assert!(html.contains("data-repos-followed=\"1\""));
    assert!(
        html.contains(">First merged PR</a>"),
        "the badge kofi awarded"
    );
    assert!(html.contains(&format!("href=\"nostr:{}\"", npub(&maya))));
    assert!(!html.contains("chip-bot"), "Maya is not a bot");
    // Pinned, in the list's order: maya's own, then the one she maintains.
    let nmea_at = html
        .find(&format!("data-repo=\"{nmea}\""))
        .expect("nmea pinned");
    let gitsail_at = html
        .find(&format!("data-repo=\"{gitsail}\""))
        .expect("gitsail pinned");
    assert!(nmea_at < gitsail_at);
    assert!(
        !html.contains(&format!("data-repo=\"{tidepool}\"")),
        "not pinned"
    );
    assert!(html.contains("Pinned by this key"));
    // Four contributions in two repos; the repo state is not one.
    assert!(
        html.contains("data-contributions=\"4\" data-repos-touched=\"2\""),
        "{html}"
    );
    assert!(html.contains("4 contributions across 2 repos in the last year"));
    assert_eq!(
        html.matches("class=\"cell l").count() + html.matches("cell future").count(),
        371 + 5
    );
    assert!(html.contains("Repo state (30618) is not counted"));
    assert!(html.contains(&format!("data-id=\"{}\"", note.id.to_hex())));
    assert!(html.contains(">Merged</span>"));

    let (_, repos) = get(relay, &format!("{base}/repos")).await;
    for repo in [&tidepool, &nmea, &gitsail] {
        assert!(repos.contains(&format!("data-repo=\"{repo}\"")), "{repo}");
    }
    assert!(repos.contains("<span class=\"chip\">maintainer</span>"));
    let (_, stars) = get(relay, &format!("{base}/starred")).await;
    assert!(stars.contains(&format!("data-repo=\"{gitsail}\"")));
    assert!(!stars.contains(&format!("data-repo=\"{tidepool}\"")));
    let (_, posts) = get(relay, &format!("{base}/posts")).await;
    assert!(posts.contains(&format!("data-id=\"{}\"", note.id.to_hex())));

    // A bot says so; a key with no pins falls back to its most-starred repos.
    let (_, bot_page) = get(relay, &format!("/social/p/{}", npub(&bot))).await;
    assert!(bot_page.contains("chip-bot"));
    let (_, kofi_page) = get(relay, &format!("/social/p/{}", npub(&kofi))).await;
    assert!(kofi_page.contains("No pinned list (kind 10617)"));
    assert!(kofi_page.contains(&format!("data-repo=\"{gitsail}\"")));

    // Every author name on /social, and the maintainers on /repo, link here.
    let (_, feed) = get(relay, "/social").await;
    assert!(feed.contains(&format!("class=\"person-link\" href=\"{base}\"")));
    let (_, code) = get(relay, &format!("/repo/{}/gitsail", npub(&kofi))).await;
    assert!(
        code.contains(&format!("class=\"person-link\" href=\"{base}\"")),
        "{code}"
    );

    for path in ["/social/p/npub1nope".to_owned(), format!("{base}/agents")] {
        assert_eq!(get(relay, &path).await.0, StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn with_social_off_repo_names_people_without_a_link() {
    let running = running_with(|name| (name == "TOON_UI_APPS").then(|| "repo".to_owned())).await;
    let relay = &running.relay;
    let owner = Keys::generate();
    store(relay, &event(&owner, 30_617, 10, "", &[&["d", "r"]])).await;
    let (status, home) = get(relay, &format!("/repo/{}/r", npub(&owner))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(!home.contains("person-link"), "{home}");
}

#[tokio::test]
async fn a_profile_shows_the_tip_split_its_kind_0_names_as_text() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let (maya, kofi) = (Keys::generate(), Keys::generate());
    let now = Timestamp::now().as_secs();
    store(relay, &event(&kofi, 0, now - 20, r#"{"name":"Kofi"}"#, &[])).await;
    store(
        relay,
        &event(
            &maya,
            0,
            now - 10,
            r#"{"name":"Maya"}"#,
            &[
                &["zap", &maya.public_key().to_hex(), "wss://r.test", "3"],
                &["zap", &kofi.public_key().to_hex(), "wss://r.test", "1"],
            ],
        ),
    )
    .await;
    let (status, page) = get(relay, &format!("/social/p/{}", npub(&maya))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("data-tip-split"), "{page}");
    assert!(page.contains("Tip split (set by this person)"), "{page}");
    assert!(page.contains("Kofi"), "{page}");
    assert!(page.contains("25%") && page.contains("75%"), "{page}");
    let (_, plain) = get(relay, &format!("/social/p/{}", npub(&kofi))).await;
    assert!(!plain.contains("data-tip-split"), "no zap tags, no card");
}

#[tokio::test]
async fn a_bots_posts_carry_a_bot_chip_in_their_by_line() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let (bot, person) = (Keys::generate(), Keys::generate());
    let now = Timestamp::now().as_secs();
    store(
        relay,
        &event(&bot, 0, now - 30, r#"{"name":"shipbot","bot":true}"#, &[]),
    )
    .await;
    store(
        relay,
        &event(&person, 0, now - 30, r#"{"name":"Maya"}"#, &[]),
    )
    .await;
    let note = event(&bot, 1, now - 20, "Release notes drafted.", &[]);
    store(relay, &note).await;
    store(relay, &event(&person, 1, now - 10, "Thanks, bot.", &[])).await;
    let (status, feed) = get(relay, "/social").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(feed.matches("chip-bot").count(), 1, "{feed}");
    let (_, post) = get(relay, &format!("/social/e/{}", note.id.to_hex())).await;
    assert!(post.contains("chip-bot"), "{post}");
}

#[tokio::test]
async fn reports_about_a_person_or_an_article_show_on_their_pages() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let (maya, viewer, friend) = (Keys::generate(), Keys::generate(), Keys::generate());
    let now = Timestamp::now().as_secs();
    let maya_hex = maya.public_key().to_hex();
    store(relay, &event(&maya, 0, now - 50, r#"{"name":"Maya"}"#, &[])).await;
    store(
        relay,
        &event(
            &maya,
            30_023,
            now - 40,
            "Body.",
            &[&["d", "post"], &["title", "Post"]],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &viewer,
            3,
            now - 30,
            "",
            &[&["p", &friend.public_key().to_hex()]],
        ),
    )
    .await;
    // A report about Maya, and one about her article (which names her too).
    store(
        relay,
        &event(
            &friend,
            1984,
            now - 20,
            "",
            &[&["p", &maya_hex, "impersonation"]],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &friend,
            1984,
            now - 10,
            "",
            &[
                &["a", &format!("30023:{maya_hex}:post"), "spam"],
                &["p", &maya_hex],
            ],
        ),
    )
    .await;
    let lens = format!("?as={}", npub(&viewer));
    let (_, profile) = get(relay, &format!("/social/p/{}{lens}", npub(&maya))).await;
    assert!(profile.contains("data-reports-banner=\"1\""), "{profile}");
    assert!(profile.contains("impersonation (1)"), "{profile}");
    assert!(
        !profile.contains("spam"),
        "the article's report is not about her"
    );
    let (_, plain) = get(relay, &format!("/social/p/{}", npub(&maya))).await;
    assert!(!plain.contains("data-reports-banner"), "no lens, no banner");
    assert!(plain.contains("data-reports=\"1\""), "{plain}");
    let (status, article) = get(relay, &format!("/social/a/{}/post{lens}", npub(&maya))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(article.contains("data-reports-banner=\"1\""), "{article}");
    assert!(article.contains("spam (1)"), "{article}");
}

#[tokio::test]
async fn a_website_alone_is_not_a_nip_39_claim() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let maya = Keys::generate();
    store(
        relay,
        &event(
            &maya,
            0,
            10,
            r#"{"name":"Maya","website":"https://maya.example"}"#,
            &[],
        ),
    )
    .await;
    let (status, page) = get(relay, &format!("/social/p/{}", npub(&maya))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("class=\"profile-website\""), "{page}");
    assert!(page.contains("href=\"https://maya.example\""), "{page}");
    assert!(
        !page.contains("Also on"),
        "no claims, no claims box: {page}"
    );
    assert!(!page.contains("NIP-39"), "{page}");
}
