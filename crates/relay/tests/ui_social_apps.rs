//! The `/social` apps, bounties and trust-and-safety pages over events
//! written through `POST /write`, driven through the router with no socket:
//! an app listing (NIP-82, draft) with its releases, files and vouchers;
//! bounties and goals; reports seen through `?as=`; and a mute list that
//! thins the feed.
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
    assert_eq!(write(relay, delivery(event)).await.0, 200);
}

fn npub(keys: &Keys) -> String {
    keys.public_key().to_bech32().expect("encodes")
}

#[tokio::test]
async fn an_app_listing_shows_its_newest_release_files_and_vouchers() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let (maya, fan, curator, viewer) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let now = Timestamp::now().as_secs();
    let repo = format!("30617:{}:tidepool", maya.public_key().to_hex());
    let app = format!("32267:{}:dev.tide.tidepool", maya.public_key().to_hex());
    let commit = "a1b2c3d4".repeat(5);
    let hash = "3f9a".repeat(16);
    store(
        relay,
        &event(&maya, 0, now - 100, r#"{"name":"Maya"}"#, &[]),
    )
    .await;
    store(
        relay,
        &event(
            &maya,
            30_617,
            now - 90,
            "",
            &[&["d", "tidepool"], &["name", "tidepool"]],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &maya,
            32_267,
            now - 80,
            "Tide charts *offline*. <script>x</script>",
            &[
                &["d", "dev.tide.tidepool"],
                &["name", "tidepool"],
                &["summary", "Offline tide charts."],
                &["icon", "https://cdn.test/icon.png"],
                &["image", "https://cdn.test/shot.png"],
                &["image", "http://cdn.test/insecure.png"],
                &["t", "sailing"],
                &["license", "MIT"],
                &["url", "https://tidepool.test"],
                &["a", &repo],
            ],
        ),
    )
    .await;
    let apk = event(
        &maya,
        3_063,
        now - 70,
        "",
        &[
            &["i", "dev.tide.tidepool"],
            &["url", "https://cdn.test/tidepool.apk"],
            &["m", "application/vnd.android.package-archive"],
            &["x", &hash],
            &["size", "8100000"],
            &["version", "1.2.0"],
            &["commit", &commit],
        ],
    );
    let wasm = event(
        &maya,
        3_063,
        now - 70,
        "",
        &[
            &["i", "dev.tide.tidepool"],
            &["m", "application/wasm"],
            &["x", &"9e12".repeat(16)],
            &["version", "1.2.0"],
            &["f", "wasm32"],
        ],
    );
    store(relay, &apk).await;
    store(relay, &wasm).await;
    let apk_id = apk.id.to_hex();
    let wasm_id = wasm.id.to_hex();
    // 1.2.0 is the newest main release though 1.1.9 was published later.
    for (at, version, channel, assets) in [
        (
            now - 60,
            "v1.2.0",
            "main",
            vec![apk_id.as_str(), wasm_id.as_str()],
        ),
        (now - 50, "1.1.9", "main", vec![]),
        (now - 40, "1.3.0-beta.1", "beta", vec![]),
    ] {
        let d = format!("dev.tide.tidepool@{version}");
        let mut tags: Vec<Vec<&str>> = vec![
            vec!["i", "dev.tide.tidepool"],
            vec!["version", version],
            vec!["d", &d],
            vec!["c", channel],
        ];
        for id in &assets {
            tags.push(vec!["e", id]);
        }
        let tags: Vec<&[&str]> = tags.iter().map(Vec::as_slice).collect();
        store(
            relay,
            &event(&maya, 30_063, at, &format!("Notes for {version}"), &tags),
        )
        .await;
    }
    store(
        relay,
        &event(&fan, 7, now - 30, "+", &[&["a", &app], &["k", "32267"]]),
    )
    .await;
    store(
        relay,
        &event(
            &curator,
            30_267,
            now - 20,
            "",
            &[&["d", "kit"], &["title", "Sailing kit"], &["a", &app]],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &viewer,
            3,
            now - 10,
            "",
            &[&["p", &curator.public_key().to_hex()]],
        ),
    )
    .await;

    let index = get(relay, "/social/apps").await;
    assert_eq!(index.0, StatusCode::OK);
    assert!(index.1.contains(&format!("data-app=\"{app}\"")));
    assert!(index.1.contains("data-vouches=\"2\""));
    assert!(index.1.contains("Sailing kit"));
    assert!(
        index
            .1
            .contains("href=\"/social/apps\" aria-current=\"page\">Apps</a>"),
        "the header links here, marked current"
    );
    let filtered = get(relay, "/social/apps?t=rust").await;
    assert!(!filtered.1.contains(&format!("data-app=\"{app}\"")));
    assert!(
        get(relay, "/social/apps?t=Sailing")
            .await
            .1
            .contains(&format!("data-app=\"{app}\""))
    );

    let path = format!("/social/app/{}/dev.tide.tidepool", npub(&maya));
    let (status, page) = get(relay, &path).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("data-release=\"v1.2.0\""), "{page}");
    assert!(page.contains("Notes for v1.2.0"));
    assert!(page.contains("NIP-82 is still an open draft"));
    assert!(!page.contains("<script>x"));
    assert!(page.contains("<img class=\"app-icon app-icon-lg\" src=\"https://cdn.test/icon.png\""));
    assert!(!page.contains("http://cdn.test/insecure.png"));
    assert!(page.contains(
        "href=\"https://cdn.test/tidepool.apk\" rel=\"nofollow noopener noreferrer\">Download"
    ));
    assert!(page.contains("Fetch by hash from Blossom"));
    assert!(page.contains(&format!("title=\"{hash}\">3f9a…3f9a</span>")));
    assert!(page.contains(&format!("/repo/{}/tidepool/commit/{commit}", npub(&maya))));
    assert!(page.contains("8.1 MB"));
    assert!(page.contains("WebAssembly 32"));
    assert!(page.contains("channel=beta"));
    assert!(page.contains("data-vouchers=\"2\""));
    assert!(page.contains("who also keeps the source repo"));
    let beta = get(relay, &format!("{path}?channel=beta")).await.1;
    assert!(beta.contains("data-release=\"1.3.0-beta.1\""));
    let naddr = get(
        relay,
        &format!(
            "/social/app/{}",
            nostr::nips::nip19::Nip19Coordinate::new(
                nostr::nips::nip01::Coordinate {
                    kind: Kind::from(32_267),
                    public_key: maya.public_key(),
                    identifier: "dev.tide.tidepool".to_owned(),
                },
                Vec::<nostr::types::RelayUrl>::new(),
            )
            .to_bech32()
            .expect("encodes")
        ),
    )
    .await;
    assert!(naddr.1.contains("data-release=\"v1.2.0\""));

    // Read as a key that follows only the curator: one voucher.
    let lens = get(relay, &format!("{path}?as={}", npub(&viewer))).await.1;
    assert!(lens.contains("data-vouchers=\"1\""), "{lens}");
    assert!(lens.contains("No reports from them."));

    // The repo page names the latest release.
    let repo_page = get(relay, &format!("/social/r/{}/tidepool", npub(&maya)))
        .await
        .1;
    assert!(repo_page.contains("data-latest-release"));
    assert!(repo_page.contains("v1.2.0"));
}

#[tokio::test]
async fn bounties_resolve_with_their_issue_and_goals_show_no_progress() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let (maya, ruth, dan, stranger) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let now = Timestamp::now().as_secs();
    let repo = format!("30617:{}:tidepool", maya.public_key().to_hex());
    store(
        relay,
        &event(
            &maya,
            30_617,
            now - 100,
            "",
            &[
                &["d", "tidepool"],
                &["name", "tidepool"],
                &["zap", &maya.public_key().to_hex(), "", "4"],
                &["zap", &dan.public_key().to_hex(), "", "1"],
            ],
        ),
    )
    .await;
    let resolved_issue = event(
        &ruth,
        1621,
        now - 90,
        "Late tides",
        &[&["a", &repo], &["subject", "Late tides"]],
    );
    let forged_issue = event(
        &ruth,
        1621,
        now - 89,
        "Wind",
        &[&["a", &repo], &["subject", "Wind"]],
    );
    store(relay, &resolved_issue).await;
    store(relay, &forged_issue).await;
    store(
        relay,
        &event(
            &maya,
            1631,
            now - 80,
            "",
            &[&["e", &resolved_issue.id.to_hex()], &["a", &repo]],
        ),
    )
    .await;
    // A stranger's resolve is not honoured.
    store(
        relay,
        &event(
            &stranger,
            1631,
            now - 80,
            "",
            &[&["e", &forged_issue.id.to_hex()], &["a", &repo]],
        ),
    )
    .await;
    let listing = |d: &str, title: &str, extra: &[&[&str]]| {
        let mut tags: Vec<&[&str]> = vec![&["t", "bounty"], &["price", "50", "USDC"]];
        tags.extend_from_slice(extra);
        let d_tag = ["d", d];
        let title_tag = ["title", title];
        tags.push(&d_tag);
        tags.push(&title_tag);
        event(&dan, 30_402, now - 70, "", &tags)
    };
    let resolved = listing(
        "b1",
        "Fix late tides",
        &[&["e", &resolved_issue.id.to_hex()]],
    );
    let open = listing("b2", "Wind overlays", &[&["e", &forged_issue.id.to_hex()]]);
    let claimed = listing("b3", "Windows build", &[&["a", &repo], &["status", "sold"]]);
    let unlinked = listing("b4", "Not about a repo", &[]);
    for each in [&resolved, &open, &claimed, &unlinked] {
        store(relay, each).await;
    }
    store(
        relay,
        &event(
            &maya,
            9041,
            now - 60,
            "Fund a year of tide data",
            &[
                &["amount", "500000000"],
                &["relays", "wss://x"],
                &["a", &repo],
            ],
        ),
    )
    .await;

    let (status, page) = get(relay, "/social/bounties").await;
    assert_eq!(status, StatusCode::OK);
    let status_of = |id: &str| {
        page.split(&format!("data-id=\"{id}\" data-bounty-status=\""))
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .map(str::to_owned)
    };
    assert_eq!(
        status_of(&resolved.id.to_hex()).as_deref(),
        Some("resolved")
    );
    assert_eq!(status_of(&open.id.to_hex()).as_deref(), Some("open"));
    assert_eq!(status_of(&claimed.id.to_hex()).as_deref(), Some("claimed"));
    assert_eq!(status_of(&unlinked.id.to_hex()), None);
    assert!(page.contains("50 USDC"));
    assert!(page.contains("500,000 sats"));
    assert!(page.contains("data-no-progress"));
    assert!(!page.contains("progress\" value"));
    let open_only = get(relay, "/social/bounties?status=open").await.1;
    assert!(open_only.contains(&open.id.to_hex()));
    assert!(!open_only.contains(&claimed.id.to_hex()));

    let repo_page = get(relay, &format!("/social/r/{}/tidepool", npub(&maya)))
        .await
        .1;
    assert!(
        repo_page.contains("data-open-bounties=\"1\""),
        "{repo_page}"
    );
    assert!(repo_page.contains("Tip split (set by the owner)"));
    assert!(repo_page.contains("80%"));
    assert!(repo_page.contains("20%"));
}

#[tokio::test]
async fn acknowledged_tips_show_as_goal_progress_bounty_payouts_and_repo_totals() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let (maya, dan, stranger) = (Keys::generate(), Keys::generate(), Keys::generate());
    let now = Timestamp::now().as_secs();
    let repo = format!("30617:{}:tidepool", maya.public_key().to_hex());
    store(
        relay,
        &event(
            &maya,
            30_617,
            now - 100,
            "",
            &[&["d", "tidepool"], &["name", "tidepool"]],
        ),
    )
    .await;
    let goal = event(
        &maya,
        9041,
        now - 90,
        "Fund tide data",
        &[&["amount", "10000000", "USDC", "6"], &["a", &repo]],
    );
    store(relay, &goal).await;
    let bounty = event(
        &maya,
        30_402,
        now - 80,
        "",
        &[
            &["d", "b1"],
            &["title", "Wind overlays"],
            &["t", "bounty"],
            &["price", "50", "USDC"],
            &["a", &repo],
        ],
    );
    store(relay, &bounty).await;
    let bounty_coordinate = format!("30402:{}:b1", maya.public_key().to_hex());
    let payer = dan.public_key().to_hex();
    let goal_id = goal.id.to_hex();
    for (signer, tags) in [
        (
            &maya,
            vec![
                vec!["e", &goal_id],
                vec!["amount", "2500000", "USDC", "6"],
                vec!["p", &payer],
                vec!["receipt", "r1"],
            ],
        ),
        (
            &maya,
            vec![
                vec!["e", &goal_id],
                vec!["amount", "1000000", "USDC", "6"],
                vec!["receipt", "r2"],
            ],
        ),
        (
            &maya,
            vec![
                vec!["e", &goal_id],
                vec!["amount", "1000000", "USDC", "6"],
                vec!["receipt", "r2"],
            ],
        ),
        (
            &maya,
            vec![
                vec!["e", &goal_id],
                vec!["amount", "900", "EUR", "2"],
                vec!["receipt", "r3"],
            ],
        ),
        (
            &stranger,
            vec![
                vec!["e", &goal_id],
                vec!["amount", "9000000", "USDC", "6"],
                vec!["receipt", "r4"],
            ],
        ),
        (
            &maya,
            vec![
                vec!["a", &bounty_coordinate],
                vec!["amount", "5000000", "USDC", "6"],
                vec!["receipt", "r5"],
            ],
        ),
        (
            &maya,
            vec![
                vec!["a", &repo],
                vec!["amount", "4000000", "USDC", "6"],
                vec!["p", &payer],
                vec!["receipt", "r6"],
            ],
        ),
    ] {
        let tags: Vec<&[&str]> = tags.iter().map(Vec::as_slice).collect();
        store(relay, &event(signer, 9851, now - 10, "", &tags)).await;
    }

    let (_, page) = get(relay, "/social/bounties").await;
    assert!(page.contains("data-progress=\"3500000\""), "{page}");
    assert!(page.contains("3.5 USDC"), "{page}");
    assert!(page.contains("1 named payer"), "{page}");
    assert!(
        !page.contains("9 USDC"),
        "a stranger's acknowledgement counts for nothing"
    );
    assert!(page.contains("data-paid"), "{page}");
    assert!(page.contains("5 USDC"), "{page}");

    let repo_page = get(relay, &format!("/social/r/{}/tidepool", npub(&maya)))
        .await
        .1;
    assert!(repo_page.contains("data-tips-received"), "{repo_page}");
    assert!(repo_page.contains("4 USDC"), "{repo_page}");
}

#[tokio::test]
async fn reports_raise_a_banner_only_through_the_lens_and_mutes_thin_the_feed() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let (maya, friend, stranger, viewer, spammer) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let now = Timestamp::now().as_secs();
    let repo = format!("30617:{}:tidepool", maya.public_key().to_hex());
    let owner = maya.public_key().to_hex();
    store(
        relay,
        &event(
            &maya,
            30_617,
            now - 100,
            "",
            &[&["d", "tidepool"], &["name", "tidepool"]],
        ),
    )
    .await;
    let ship = event(
        &maya,
        1,
        now - 95,
        "Shipped tidepool.",
        &[&["a", &repo], &["t", "ship"]],
    );
    store(relay, &ship).await;
    store(
        relay,
        &event(
            &stranger,
            1984,
            now - 90,
            "",
            &[&["a", &repo, "spam"], &["p", &owner]],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &friend,
            1984,
            now - 80,
            "Bundles a miner.",
            &[&["a", &repo, "malware"], &["p", &owner]],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &viewer,
            3,
            now - 70,
            "",
            &[&["p", &friend.public_key().to_hex()]],
        ),
    )
    .await;

    let base = format!("/social/r/{}/tidepool", npub(&maya));
    let plain = get(relay, &base).await.1;
    assert!(!plain.contains("data-reports-banner"), "no lens, no banner");
    assert!(plain.contains("data-reports=\"2\""));
    let lens = get(relay, &format!("{base}?as={}", npub(&viewer))).await.1;
    assert!(lens.contains("data-reports-banner=\"1\""), "{lens}");
    assert!(lens.contains("malware (1)"));
    assert!(lens.contains(&format!("{base}/reports?as={}", npub(&viewer))));
    let list = get(relay, &format!("{base}/reports?as={}", npub(&viewer))).await;
    assert_eq!(list.0, StatusCode::OK);
    assert!(list.1.contains("Bundles a miner."));
    assert!(list.1.contains("data-reports-from-follows=\"1\""));
    // The launch carries the repo's banner too.
    let post = get(
        relay,
        &format!("/social/e/{}?as={}", ship.id.to_hex(), npub(&viewer)),
    )
    .await
    .1;
    assert!(post.contains("data-reports-banner=\"1\""));

    // A mute list hides the spammer and a hashtag, only for its owner's view.
    let spam = event(&spammer, 1, now - 60, "Free tokens", &[]);
    let tagged = event(&friend, 1, now - 50, "Airdrop soon", &[&["t", "airdrop"]]);
    let kept = event(&friend, 1, now - 40, "Fixed the parser", &[]);
    for each in [&spam, &tagged, &kept] {
        store(relay, each).await;
    }
    store(
        relay,
        &event(
            &viewer,
            10_000,
            now - 30,
            "",
            &[&["p", &spammer.public_key().to_hex()], &["t", "airdrop"]],
        ),
    )
    .await;
    let everyone = get(relay, "/social").await.1;
    for each in [&spam, &tagged, &kept] {
        assert!(everyone.contains(&each.id.to_hex()));
    }
    let muted = get(relay, &format!("/social?as={}", npub(&viewer))).await.1;
    assert!(!muted.contains(&spam.id.to_hex()));
    assert!(!muted.contains(&tagged.id.to_hex()));
    assert!(muted.contains(&kept.id.to_hex()));
    assert!(muted.contains("data-muted"));
    let following = get(relay, &format!("/social/following/{}", npub(&viewer)))
        .await
        .1;
    assert!(following.contains(&kept.id.to_hex()));
    assert!(!following.contains(&tagged.id.to_hex()));
}
