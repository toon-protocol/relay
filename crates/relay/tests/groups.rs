//! NIP-29 over a real socket and the write router (#219): off unless the
//! operator turns it on, membership judged by the event's author, metadata
//! the relay signs, and closed groups read by a member who has authenticated.

mod common;

use common::{Client, delivery, running, running_with, signed_by, write};
use nostr::event::Event;
use nostr::key::Keys;
use serde_json::{Value, json};

const NOW: u64 = 1_700_000_000;

async fn with_groups() -> common::Running {
    running_with(|name| (name == "TOON_NIP29_GROUPS").then(|| "true".to_string())).await
}

fn in_group(keys: &Keys, kind: u16, group: &str, extra: &[&[&str]]) -> Event {
    let head = ["h", group];
    let mut tags: Vec<&[&str]> = vec![&head[..]];
    tags.extend_from_slice(extra);
    signed_by(keys, kind, NOW, &tags)
}

async fn status(running: &common::Running, event: &Event) -> u16 {
    write(&running.relay, delivery(event)).await.0
}

async fn accepted(running: &common::Running, event: &Event) {
    assert_eq!(status(running, event).await, 200, "{event:?}");
}

async fn authenticate(client: &mut Client, keys: &Keys) {
    let frame = client.next().await.expect("a challenge arrives first");
    let challenge = frame[1].as_str().expect("text").to_string();
    let auth = signed_by(
        keys,
        22242,
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after the epoch")
            .as_secs(),
        &[&["relay", "ws://relay.test"], &["challenge", &challenge]],
    );
    client.send(json!(["AUTH", auth])).await;
    let ok = client.next().await.expect("an OK arrives");
    assert_eq!(ok[2], true, "{ok}");
}

fn kinds(events: &[Value]) -> Vec<u64> {
    let mut kinds: Vec<u64> = events.iter().filter_map(|e| e["kind"].as_u64()).collect();
    kinds.sort_unstable();
    kinds
}

#[tokio::test]
async fn by_default_an_h_tag_is_any_tag() {
    let running = running().await;
    let event = in_group(&Keys::generate(), 9, "nowhere", &[]);
    accepted(&running, &event).await;
}

#[tokio::test]
async fn a_group_is_made_publishes_its_metadata_and_admits_members_only() {
    let running = with_groups().await;
    let (owner, member, outsider) = (Keys::generate(), Keys::generate(), Keys::generate());
    accepted(
        &running,
        &in_group(&owner, 9007, "club", &[&["name", "Agents"]]),
    )
    .await;

    let mut client = Client::connect(&running.read_url).await;
    client.next().await.expect("the NIP-42 challenge");
    let found = client
        .req(
            "meta",
            json!({ "kinds": [39000, 39001, 39002, 39003], "#d": ["club"] }),
        )
        .await;
    assert_eq!(kinds(&found), [39000, 39001, 39002, 39003]);
    let relay_key =
        Keys::new(nostr::key::SecretKey::from_hex(&"1".repeat(64)).expect("a secret key"))
            .public_key()
            .to_hex();
    assert!(found.iter().all(|e| e["pubkey"] == relay_key));

    assert_eq!(
        status(&running, &in_group(&outsider, 9, "club", &[])).await,
        403
    );
    let add = in_group(
        &owner,
        9000,
        "club",
        &[&["p", &member.public_key().to_hex()]],
    );
    accepted(&running, &add).await;
    accepted(&running, &in_group(&member, 9, "club", &[])).await;
    assert_eq!(
        status(&running, &in_group(&outsider, 9, "club", &[])).await,
        403
    );
    assert_eq!(
        status(&running, &in_group(&member, 9007, "club", &[])).await,
        409
    );
    assert_eq!(
        status(&running, &in_group(&member, 9, "missing", &[])).await,
        404
    );
    assert_eq!(
        status(
            &running,
            &in_group(&owner, 39000, "club", &[&["d", "club"]])
        )
        .await,
        403
    );
}

#[tokio::test]
async fn a_closed_group_is_read_by_an_authenticated_member_only() {
    let running = with_groups().await;
    let (owner, stranger) = (Keys::generate(), Keys::generate());
    accepted(&running, &in_group(&owner, 9007, "shut", &[&["closed"]])).await;
    let message = in_group(&owner, 9, "shut", &[]);
    accepted(&running, &message).await;

    let mut anonymous = Client::connect(&running.read_url).await;
    anonymous.next().await.expect("the challenge");
    anonymous
        .send(json!(["REQ", "g", { "#h": ["shut"] }]))
        .await;
    let closed = anonymous.next().await.expect("a CLOSED");
    assert!(
        closed[2]
            .as_str()
            .unwrap_or_default()
            .starts_with("auth-required:")
    );
    // A filter that does not name the group is answered without its events.
    assert!(
        anonymous
            .req("sweep", json!({ "kinds": [9] }))
            .await
            .is_empty()
    );

    let mut outsider = Client::connect(&running.read_url).await;
    authenticate(&mut outsider, &stranger).await;
    outsider.send(json!(["REQ", "g", { "#h": ["shut"] }])).await;
    let closed = outsider.next().await.expect("a CLOSED");
    assert!(
        closed[2]
            .as_str()
            .unwrap_or_default()
            .starts_with("restricted:")
    );

    let mut member = Client::connect(&running.read_url).await;
    authenticate(&mut member, &owner).await;
    let found = member.req("g", json!({ "#h": ["shut"] })).await;
    assert_eq!(found.iter().filter(|e| e["kind"] == 9).count(), 1);
}

#[tokio::test]
async fn the_groups_come_back_when_the_relay_is_opened_again() {
    let running = with_groups().await;
    let (owner, outsider) = (Keys::generate(), Keys::generate());
    accepted(&running, &in_group(&owner, 9007, "kept", &[])).await;

    let config = relay::Config::from_env(|name| match name {
        "TOON_SECRET_KEY" => Some("1".repeat(64)),
        "TOON_NIP29_GROUPS" => Some("true".to_string()),
        "TOON_DATA_DIR" => running
            .database
            .parent()
            .map(|dir| dir.to_string_lossy().into_owned()),
        _ => None,
    })
    .expect("the same configuration");
    let reopened = relay::Relay::open(&config).expect("the file opens again");
    assert_eq!(
        write_to(&reopened, &in_group(&outsider, 9, "kept", &[])).await,
        403
    );
    assert_eq!(
        write_to(&reopened, &in_group(&owner, 9, "kept", &[])).await,
        200
    );
}

async fn write_to(relay: &relay::Relay, event: &Event) -> u16 {
    write(relay, delivery(event)).await.0
}
