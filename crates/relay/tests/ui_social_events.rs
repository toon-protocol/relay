//! The `/social` events and rooms pages over events written through
//! `POST /write` with NIP-29 on, driven through the router with no socket:
//! a calendar event with its RSVPs, gallery, badges and live stream, its
//! room, a repo's room, and a private group that no page shows.
#![cfg(feature = "ui-social")]

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
    assert_eq!(write(relay, delivery(event)).await.0, 200, "{event:?}");
}

fn npub(keys: &Keys) -> String {
    keys.public_key().to_bech32().expect("encodes")
}

#[tokio::test]
async fn an_event_page_reads_its_rsvps_gallery_badges_stream_and_room() {
    let running =
        running_with(|name| (name == "TOON_NIP29_GROUPS").then(|| "true".to_owned())).await;
    let relay = &running.relay;
    let (host, maya, leo, fan) = (
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
        Keys::generate(),
    );
    let now = Timestamp::now().as_secs();
    let at = |ago: u64| now - ago;
    let hex = |keys: &Keys| keys.public_key().to_hex();

    store(relay, &event(&host, 0, at(500), r#"{"name":"Iris"}"#, &[])).await;
    store(relay, &event(&maya, 0, at(500), r#"{"name":"Maya"}"#, &[])).await;
    // The event's room: open and public.
    store(
        relay,
        &event(
            &host,
            9007,
            at(400),
            "",
            &[&["h", "jam"], &["name", "Jam room"]],
        ),
    )
    .await;
    store(relay, &event(&maya, 9021, at(399), "", &[&["h", "jam"]])).await;
    store(
        relay,
        &event(
            &maya,
            9,
            at(398),
            "Doors open, post early",
            &[&["h", "jam"]],
        ),
    )
    .await;

    // A hackathon that ended a minute ago, posted into its room.
    let (start, end) = ((now - 7_200).to_string(), (now - 60).to_string());
    let calendar = event(
        &host,
        31_923,
        at(390),
        "Ship something people can run.",
        &[
            &["d", "jam"],
            &["title", "Weekend <Jam>"],
            &["start", &start],
            &["end", &end],
            &["start_tzid", "Europe/Berlin"],
            &["location", "online"],
            &["h", "jam"],
        ],
    );
    store(relay, &calendar).await;
    let coordinate = format!("31923:{}:jam", hex(&host));

    // RSVPs: two people going, one changed their mind.
    for (keys, d, ago, status) in [
        (&maya, "a", 380, "accepted"),
        (&leo, "b", 380, "accepted"),
        (&fan, "c", 380, "accepted"),
        (&fan, "c", 370, "declined"),
    ] {
        store(
            relay,
            &event(
                keys,
                31_925,
                at(ago),
                "",
                &[&["d", d], &["a", &coordinate], &["status", status]],
            ),
        )
        .await;
    }

    // Submissions: Leo's repo, Maya's repo and her ship post naming it.
    let maya_repo = format!("30617:{}:tidepool", hex(&maya));
    store(
        relay,
        &event(
            &leo,
            30_617,
            at(360),
            "",
            &[
                &["d", "forge"],
                &["name", "pixel-forge"],
                &["a", &coordinate],
            ],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &maya,
            30_617,
            at(350),
            "",
            &[&["d", "tidepool"], &["name", "tidepool"]],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &maya,
            1,
            at(340),
            "Shipped tidepool for the jam.",
            &[&["a", &maya_repo], &["a", &coordinate], &["t", "ship"]],
        ),
    )
    .await;
    // A plain note that names the event is talk, not a submission.
    let talk = event(&fan, 1, at(339), "good luck all", &[&["a", &coordinate]]);
    store(relay, &talk).await;

    // The host's badge, and its award to Maya; a stranger's award is no prize.
    store(
        relay,
        &event(
            &host,
            30_009,
            at(330),
            "",
            &[&["d", "winner"], &["name", "Winner"], &["a", &coordinate]],
        ),
    )
    .await;
    let badge = format!("30009:{}:winner", hex(&host));
    store(
        relay,
        &event(
            &host,
            8,
            at(320),
            "",
            &[&["a", &badge], &["p", &hex(&maya)]],
        ),
    )
    .await;
    store(
        relay,
        &event(&fan, 8, at(319), "", &[&["a", &badge], &["p", &hex(&leo)]]),
    )
    .await;

    // A live stream of the closing ceremony.
    store(
        relay,
        &event(
            &host,
            30_311,
            at(10),
            "",
            &[
                &["d", "closing"],
                &["title", "Closing ceremony"],
                &["status", "live"],
                &["streaming", "https://stream.test/live.m3u8"],
                &["current_participants", "36"],
                &["a", &coordinate],
            ],
        ),
    )
    .await;

    let path = format!("/social/ev/{}/jam", npub(&host));
    let (status, page) = get(relay, &path).await;
    assert_eq!(status, StatusCode::OK, "{page}");
    assert!(page.contains("Weekend &lt;Jam&gt;"));
    assert!(page.contains("data-phase=\"ended\""));
    assert!(page.contains("the host&#39;s time zone is Europe/Berlin"));
    assert!(page.contains("2 going &middot; 0 interested"), "{page}");
    assert!(
        page.contains("href=\"nostr:naddr1"),
        "RSVP links to a client"
    );
    assert!(page.contains("2 projects shipped"), "{page}");
    assert!(page.contains("Winners pinned first"));
    let pinned = page
        .find(&format!("data-id=\"{maya_repo}\" data-winner"))
        .expect("Maya's repo is a winner");
    let other = page
        .find(&format!("data-id=\"30617:{}:forge\"", hex(&leo)))
        .expect("Leo's repo is in the gallery");
    assert!(pinned < other, "the winner is pinned first");
    assert!(
        !page.contains(&talk.id.to_hex()),
        "talk is not a submission"
    );
    assert!(
        page.contains("awarded to 1 person"),
        "a stranger's award does not count"
    );
    assert!(page.contains("Closing ceremony &middot; 36 watching"));
    assert!(page.contains("href=\"https://stream.test/live.m3u8\""));
    assert!(page.contains("data-room=\"jam\""), "{page}");
    assert!(page.contains("Doors open, post early"));
    assert!(page.contains("Read-only here; chat from your client"));

    let (status, index) = get(relay, "/social/events").await;
    assert_eq!(status, StatusCode::OK);
    assert!(index.contains(&format!("data-event=\"{coordinate}\"")));
    let (_, feed) = get(relay, "/social").await;
    assert!(feed.contains("data-coming-up"));
    assert!(
        feed.contains("href=\"/social/events\""),
        "the header links Events"
    );

    let (status, room) = get(relay, "/social/g/jam").await;
    assert_eq!(status, StatusCode::OK);
    assert!(room.contains("#Jam room"));
    assert!(room.contains("Doors open, post early"));
    assert!(room.contains("Read-only here; chat from your client"));
}

#[tokio::test]
async fn a_private_room_shows_nothing_and_a_repo_links_its_room() {
    let running =
        running_with(|name| (name == "TOON_NIP29_GROUPS").then(|| "true".to_owned())).await;
    let relay = &running.relay;
    let maya = Keys::generate();
    let now = Timestamp::now().as_secs();

    store(
        relay,
        &event(
            &maya,
            9007,
            now - 100,
            "",
            &[&["h", "core"], &["name", "Core team"], &["private"]],
        ),
    )
    .await;
    let secret = event(
        &maya,
        9,
        now - 90,
        "the launch date is secret",
        &[&["h", "core"]],
    );
    store(relay, &secret).await;
    let secret_note = event(&maya, 1, now - 89, "members only note", &[&["h", "core"]]);
    store(relay, &secret_note).await;

    // The relay serves a visitor no private group's metadata, so its room
    // is not found, as a room that does not exist is.
    let (status, room) = get(relay, "/social/g/core").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(room.contains("needs sign-in in a client"));
    assert!(
        !room.contains("Core team"),
        "a private room's name is not shown"
    );
    assert!(!room.contains("secret"));
    let (status, _) = get(relay, &format!("/social/e/{}", secret.id.to_hex())).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a private message has no page"
    );
    let (_, feed) = get(relay, "/social").await;
    assert!(
        !feed.contains("members only note"),
        "nor does it reach the feed"
    );

    // A repo's room is the group named its d that its maker keeps.
    store(
        relay,
        &event(
            &maya,
            30_617,
            now - 80,
            "",
            &[&["d", "tidepool"], &["name", "tidepool"]],
        ),
    )
    .await;
    store(
        relay,
        &event(
            &maya,
            9007,
            now - 70,
            "",
            &[&["h", "tidepool"], &["name", "tidepool"]],
        ),
    )
    .await;
    store(
        relay,
        &event(&maya, 9, now - 60, "welcome", &[&["h", "tidepool"]]),
    )
    .await;
    let (status, page) = get(relay, &format!("/social/r/{}/tidepool", npub(&maya))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("data-room=\"tidepool\""), "{page}");
    assert!(page.contains("1 message today"));
    assert!(page.contains("href=\"/social/g/tidepool\""));
}

#[tokio::test]
async fn without_groups_there_are_no_rooms() {
    let running = running_with(|_| None).await;
    let (status, page) = get(&running.relay, "/social/g/anything").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert!(page.contains("This relay has no rooms"));
}
