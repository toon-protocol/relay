//! The `/social` pages over events written through `POST /write`, driven
//! through the router with no socket: the feed, the repo card, the repo
//! page's tabs, a ship post, and the links between `/repo` and `/social`.
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

const POLICY: &str = "default-src 'self'; script-src 'none'; img-src 'self' https:; \
                      media-src 'self' https:; form-action 'self'; base-uri 'none'";

struct Page {
    status: StatusCode,
    policy: Option<String>,
    referrer: Option<String>,
    text: String,
}

async fn get(relay: &relay::Relay, path: &str) -> Page {
    let response = relay
        .read_router()
        .oneshot(Request::get(path).body(Body::empty()).expect("a request"))
        .await
        .expect("infallible");
    let header = |name: &str| {
        response
            .headers()
            .get(name)
            .map(|value| value.to_str().expect("text").to_owned())
    };
    let (status, policy, referrer) = (
        response.status(),
        header("content-security-policy"),
        header("referrer-policy"),
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
        referrer,
        text: String::from_utf8_lossy(&bytes).into_owned(),
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

fn npub(keys: &Keys) -> String {
    keys.public_key().to_bech32().expect("encodes")
}

#[tokio::test]
async fn the_social_pages_read_written_events() {
    let running = running_with(|name| match name {
        "TOON_UI_RELAY_URL" => Some("wss://relay.test".to_owned()),
        "TOON_UI_NSITE_GATEWAY" => Some("https://nsite.test".to_owned()),
        _ => None,
    })
    .await;
    let relay = &running.relay;
    let owner = Keys::generate();
    let maintainer = Keys::generate();
    let (fan, other_fan, stranger) = (Keys::generate(), Keys::generate(), Keys::generate());
    let now = Timestamp::now().as_secs();
    let a = format!("30617:{}:tidepool", owner.public_key().to_hex());
    let maintainer_hex = maintainer.public_key().to_hex();

    store(
        relay,
        &event(
            &owner,
            0,
            now - 100,
            r#"{"name":"Maya","nip05":"maya@tide.dev"}"#,
            &[],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &owner,
            30_617,
            now - 90,
            "",
            &[
                &["d", "tidepool"],
                &["name", "tidepool"],
                &["description", "Tide <script>alert(1)</script> charts"],
                &["t", "rust"],
                &["t", "sailing"],
                &["maintainers", &maintainer_hex],
            ],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &maintainer,
            30_618,
            now - 80,
            "",
            &[
                &["d", "tidepool"],
                &["HEAD", "ref: refs/heads/main"],
                &["refs/heads/main", &"a".repeat(40)],
            ],
        ),
    )
    .await;
    // Stars: two people, one of them twice; a dislike and a reaction without
    // the kind are not stars. The second fan then deletes theirs.
    for (keys, content, tags) in [
        (&fan, "+", vec![vec!["a", a.as_str()], vec!["k", "30617"]]),
        (&fan, "+", vec![vec!["a", a.as_str()], vec!["k", "30617"]]),
        (
            &stranger,
            "-",
            vec![vec!["a", a.as_str()], vec!["k", "30617"]],
        ),
        (&stranger, "+", vec![vec!["a", a.as_str()]]),
    ] {
        let tags: Vec<&[&str]> = tags.iter().map(Vec::as_slice).collect();
        store(relay, &event(keys, 7, now - 70, content, &tags)).await;
    }
    let unstarred = event(&other_fan, 7, now - 69, "+", &[&["a", &a], &["k", "30617"]]);
    store(relay, &unstarred).await;
    store(
        relay,
        &event(
            &other_fan,
            5,
            now - 68,
            "",
            &[&["e", &unstarred.id.to_hex()], &["k", "7"]],
        ),
    )
    .await;
    store(relay, &event(&fan, 10_018, now - 60, "", &[&["a", &a]])).await;
    store(
        relay,
        &event(
            &fan,
            3,
            now - 60,
            "",
            &[&["p", &owner.public_key().to_hex()]],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &fan,
            1621,
            now - 55,
            "Southern stations",
            &[&["a", &a], &["subject", "South"]],
        ),
    )
    .await;
    let ship = event(
        &owner,
        1,
        now - 50,
        "Shipped tidepool! https://cdn.test/shot.png",
        &[
            &["a", &a],
            &["t", "ship"],
            &[
                "imeta",
                "url https://cdn.test/shot.png",
                "m image/png",
                "alt the chart",
            ],
            &["imeta", "url http://cdn.test/insecure.png", "m image/png"],
        ],
    );
    store(relay, &ship).await;
    store(
        relay,
        &event(
            &owner,
            35_128,
            now - 45,
            "",
            &[
                &["d", "tidepool"],
                &["path", "/index.html", &"f".repeat(64)],
                &[
                    "source",
                    &format!("nostr://{}/relay.test/tidepool", npub(&owner)),
                ],
            ],
        ),
    )
    .await;
    let naddr = nostr::nips::nip19::Nip19Coordinate::new(
        nostr::nips::nip01::Coordinate::new(Kind::from(30_617), owner.public_key())
            .identifier("tidepool"),
        [],
    )
    .to_bech32()
    .expect("encodes");
    let mention = event(&fan, 1, now - 40, &format!("Look: nostr:{naddr}"), &[]);
    store(relay, &mention).await;
    let reply = event(
        &fan,
        1,
        now - 30,
        "Works offline at the dock",
        &[&["e", &ship.id.to_hex(), "", "root"]],
    );
    store(relay, &reply).await;
    let comment = event(
        &stranger,
        1111,
        now - 20,
        "Wind overlays next?",
        &[&["A", &a], &["K", "30617"], &["a", &a], &["k", "30617"]],
    );
    store(relay, &comment).await;

    // The feed: the ship post as a launch, the mention unfurled.
    let feed = get(relay, "/social").await;
    assert_eq!(feed.status, StatusCode::OK);
    assert_eq!(feed.policy.as_deref(), Some(POLICY));
    assert_eq!(feed.referrer.as_deref(), Some("no-referrer"));
    let html = &feed.text;
    assert!(!html.contains("<script"), "{html}");
    assert!(html.contains(&format!("data-id=\"{}\" data-ship", ship.id.to_hex())));
    assert!(html.contains("<img class=\"hero\" src=\"https://cdn.test/shot.png\""));
    assert!(!html.contains("http://cdn.test/insecure.png"));
    assert!(html.contains(&format!("data-id=\"{}\"", mention.id.to_hex())));
    assert!(
        !html.contains(&format!("data-id=\"{}\"", reply.id.to_hex())),
        "a reply is on its post's page"
    );
    assert!(html.contains(&format!("data-repo=\"{a}\"")));
    assert!(
        html.contains("data-stars=\"1\""),
        "one person, and the deleted star is gone"
    );
    assert!(html.contains("data-followers=\"1\""));
    assert!(html.contains("1 open issue"));
    assert!(html.contains(&format!(
        "git clone nostr://{}/relay.test/tidepool",
        npub(&owner)
    )));
    assert!(html.contains("git clone nostr://maya@tide.dev/relay.test/tidepool"));
    assert!(html.contains(".nsite.test/\" rel=\"noopener noreferrer\" data-demo"));
    assert!(html.contains("Trending today"));

    // The repo page and its tabs.
    let base = format!("/social/r/{}/tidepool", npub(&owner));
    let maker = get(relay, &base).await;
    assert_eq!(maker.status, StatusCode::OK);
    assert_eq!(maker.policy.as_deref(), Some(POLICY));
    assert!(
        maker
            .text
            .contains(&format!("data-id=\"{}\"", ship.id.to_hex()))
    );
    assert!(
        !maker
            .text
            .contains(&format!("data-id=\"{}\"", comment.id.to_hex()))
    );
    assert!(
        maker
            .text
            .contains(&format!("href=\"/repo/{}/tidepool\"", npub(&owner))),
        "Code goes back to /repo"
    );
    assert!(maker.text.contains("Tide &lt;script&gt;"));
    let talk = get(relay, &format!("{base}/talk")).await;
    assert!(
        talk.text
            .contains(&format!("data-id=\"{}\"", comment.id.to_hex()))
    );
    let stars = get(relay, &format!("{base}/stars")).await;
    assert!(stars.text.contains(&npub(&fan)));
    assert!(!stars.text.contains(&npub(&other_fan)));
    assert!(!stars.text.contains(&npub(&stranger)));
    let followers = get(relay, &format!("{base}/followers")).await;
    assert!(followers.text.contains(&npub(&fan)));
    for tab in ["media", "snippets"] {
        assert_eq!(
            get(relay, &format!("{base}/{tab}")).await.status,
            StatusCode::OK
        );
    }
    let forks = get(relay, &format!("{base}/forks")).await;
    assert_eq!(forks.status, StatusCode::OK);
    assert!(
        forks
            .text
            .contains("No forks or remixes of this repo on this relay yet.")
    );
    assert!(forks.text.contains("public and permanent"));
    assert_eq!(
        get(relay, &format!("{base}/lineage")).await.status,
        StatusCode::NOT_FOUND
    );

    // The ship post's page.
    let launch = get(relay, &format!("/social/e/{}", ship.id.to_hex())).await;
    assert_eq!(launch.status, StatusCode::OK);
    assert!(launch.text.contains("launch-title"));
    assert!(launch.text.contains("owns this repo"));
    assert!(
        launch
            .text
            .contains(&format!("data-id=\"{}\"", reply.id.to_hex()))
    );

    // The following feed of the fan, who follows the owner.
    let following = get(relay, &format!("/social/following/{}", npub(&fan))).await;
    assert_eq!(following.status, StatusCode::OK);
    assert!(
        following
            .text
            .contains(&format!("data-id=\"{}\"", ship.id.to_hex()))
    );
    assert!(following.text.contains("opened an issue"));

    // /repo links to the repo's social page, and back.
    let home = get(relay, &format!("/repo/{}/tidepool", npub(&owner))).await;
    assert!(
        home.text.contains(&format!("href=\"{base}\"")),
        "{}",
        home.text
    );
    assert!(
        home.text.contains("href=\"/social\""),
        "the switcher offers Social"
    );

    for path in [
        "/social/e/not-an-id".to_owned(),
        format!("/social/e/{}", "0".repeat(64)),
        format!("/social/r/{}/none", npub(&owner)),
        "/social/nope".to_owned(),
    ] {
        let page = get(relay, &path).await;
        assert_eq!(page.status, StatusCode::NOT_FOUND, "{path}");
        assert!(!page.text.contains("<script"));
    }
}

#[tokio::test]
async fn with_social_off_repo_draws_no_link_to_it() {
    let running = running_with(|name| (name == "TOON_UI_APPS").then(|| "repo".to_owned())).await;
    let relay = &running.relay;
    let owner = Keys::generate();
    store(relay, &event(&owner, 30_617, 10, "", &[&["d", "r"]])).await;
    let home = get(relay, &format!("/repo/{}/r", npub(&owner))).await;
    assert_eq!(home.status, StatusCode::OK);
    assert!(!home.text.contains("/social"), "{}", home.text);
    assert_eq!(
        get(relay, "/social").await.status,
        StatusCode::UPGRADE_REQUIRED
    );
}

#[tokio::test]
async fn forks_remixes_and_open_in_are_read_from_written_events() {
    let running = running_with(|name| match name {
        "TOON_UI_RELAY_URL" => Some("wss://relay.test".to_owned()),
        "TOON_UI_NSITE_GATEWAY" => Some("https://nsite.test".to_owned()),
        _ => None,
    })
    .await;
    let relay = &running.relay;
    let (noaa, maya, ruth, dan, leo) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let (builder, viewer) = (Keys::generate(), Keys::generate());
    let now = Timestamp::now().as_secs();
    let origin = format!("30617:{}:harmonics", noaa.public_key().to_hex());
    let tidepool = format!("30617:{}:tidepool", maya.public_key().to_hex());
    let euc = "c".repeat(40);
    let upstream = format!("{origin}|https://code.test/harmonics.git");
    let announcements: [(&Keys, &str, Vec<Vec<&str>>); 4] = [
        (&noaa, "harmonics", vec![]),
        (
            &maya,
            "tidepool",
            vec![vec!["u", &upstream], vec!["r", &euc, "euc"]],
        ),
        (&ruth, "tidepool", vec![vec!["u", &tidepool]]),
        (&dan, "tidepool-nz", vec![vec!["r", &euc, "euc"]]),
    ];
    for (keys, d, extra) in &announcements {
        let d_tag = ["d", *d];
        let mut tags: Vec<&[&str]> = vec![&d_tag];
        tags.extend(extra.iter().map(Vec::as_slice));
        store(relay, &event(keys, 30_617, now - 100, "", &tags)).await;
    }
    // Maya's site, and Leo's remix of it.
    let site = format!("35128:{}:tidepool", maya.public_key().to_hex());
    let source = format!("nostr://{}/relay.test/tidepool", npub(&maya));
    store(
        relay,
        &event(
            &maya,
            35_128,
            now - 90,
            "",
            &[
                &["d", "tidepool"],
                &["path", "/index.html", &"f".repeat(64)],
                &["source", &source],
            ],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &leo,
            35_128,
            now - 80,
            "",
            &[
                &["d", "surfcast"],
                &["a", &site],
                &["A", &site],
                &["title", "surfcast"],
                &["path", "/index.html", &"e".repeat(64)],
            ],
        ),
    )
    .await;
    // A builder that clones nostr:// repos, recommended by someone the
    // viewer follows.
    let handler = event(
        &builder,
        31_990,
        now - 70,
        r#"{"name":"Shakespeare"}"#,
        &[
            &["d", "sk"],
            &["k", "30617"],
            &["web", "https://shakespeare.diy/<bech32>", "naddr"],
        ],
    );
    store(relay, &handler).await;
    let handler_a = format!("31990:{}:sk", builder.public_key().to_hex());
    store(
        relay,
        &event(
            &dan,
            31_989,
            now - 60,
            "",
            &[&["d", "30617"], &["a", &handler_a]],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &viewer,
            3,
            now - 50,
            "",
            &[&["p", &dan.public_key().to_hex()]],
        ),
    )
    .await;

    let base = format!("/social/r/{}/tidepool", npub(&maya));
    let forks = get(relay, &format!("{base}/forks?as={}", npub(&viewer))).await;
    assert_eq!(forks.status, StatusCode::OK);
    let html = &forks.text;
    assert!(!html.contains("<script"));
    // The origin, this repo, Ruth's fork, Leo's remix, Dan's same first commit.
    for needle in [
        format!("data-repo-node=\"{origin}\""),
        format!(
            "data-repo-node=\"30617:{}:tidepool\"",
            ruth.public_key().to_hex()
        ),
        format!("data-site=\"35128:{}:surfcast\"", leo.public_key().to_hex()),
        format!(
            "data-repo-node=\"30617:{}:tidepool-nz\"",
            dan.public_key().to_hex()
        ),
    ] {
        assert!(html.contains(&needle), "{needle}");
    }
    assert!(html.contains("aria-current=\"true\""));
    assert!(html.contains(">same first commit<"));
    assert!(
        html.contains("Forks<span class=\"count\">3</span>"),
        "ruth, dan and leo"
    );
    assert!(html.contains(&format!("Forked from <a href=\"{base}/forks\">")));
    // Remix and Open in, from the recommended handler, the naddr filled in.
    assert!(html.contains("https://shakespeare.diy/clone?url=nostr%3A%2F%2F"));
    assert!(html.contains("href=\"https://shakespeare.diy/naddr1"));
    assert!(html.contains("1 you follow"));
    assert!(html.contains("public and permanent"));
    // The tab links keep the viewer.
    assert!(html.contains(&format!("{base}/talk?as={}", npub(&viewer))));

    // /repo says what it was forked from too.
    let home = get(relay, &format!("/repo/{}/tidepool", npub(&maya))).await;
    assert!(home.text.contains(&format!(
        "Forked from <a href=\"/repo/{}/harmonics\">",
        npub(&noaa)
    )));
}
