//! What the store stops serving, and what it will not keep: NIP-09 deletion
//! and its tombstones, NIP-40 expiry and the reaper, and the operator
//! blocklist. Each is checked on the file, which the TypeScript relay
//! reads too.

mod common;

use std::collections::BTreeSet;
use std::time::{SystemTime, UNIX_EPOCH};

use common::{delivery, running_with, signed, signed_by, typescript_database, verified, write};
use nostr::event::Event;
use nostr::filter::Filter;
use nostr::key::Keys;
use relay::{Retention, Saved, Store};
use rusqlite::Connection;
use tempfile::tempdir;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the test host's clock is after 1970")
        .as_secs()
}

fn ids(events: &[Event]) -> BTreeSet<String> {
    events.iter().map(|event| event.id.to_hex()).collect()
}

fn id_set(events: &[&Event]) -> BTreeSet<String> {
    events.iter().map(|event| event.id.to_hex()).collect()
}

async fn open() -> (Store, std::path::PathBuf, tempfile::TempDir) {
    let dir = tempdir().expect("a temp dir");
    let path = typescript_database(dir.path());
    (Store::open(&path).expect("it opens"), path, dir)
}

async fn held(store: &Store, keys: &Keys) -> BTreeSet<String> {
    let found = store
        .query(Filter::new().author(keys.public_key()))
        .await
        .expect("the query runs");
    ids(&found)
}

#[tokio::test]
async fn a_kind_5_removes_the_authors_event_by_id_and_it_stays_removed() {
    let (store, path, _dir) = open().await;
    let keys = Keys::generate();
    let t = now();
    let target = signed_by(&keys, 1, t - 10, &[]);
    let keeper = signed_by(&keys, 1, t - 10, &[&["t", "keeper"]]);
    store.save(&verified(&target)).await.expect("saved");
    store.save(&verified(&keeper)).await.expect("saved");

    let id = target.id.to_hex();
    let deletion = signed_by(&keys, 5, t, &[&["e", &id]]);
    assert_eq!(
        store.save(&verified(&deletion)).await.expect("saved"),
        Saved::New
    );
    // The request is stored beside what it left.
    assert_eq!(
        held(&store, &keys).await,
        id_set(&[&keeper, &deletion]),
        "the target is gone, the request and the keeper are kept"
    );

    assert_eq!(
        store.save(&verified(&target)).await.expect("answered"),
        Saved::Dropped
    );
    assert_eq!(held(&store, &keys).await, id_set(&[&keeper, &deletion]));

    let tombstone: (String, i64) = Connection::open(&path)
        .expect("opens")
        .query_row(
            "SELECT pubkey, deleted_at FROM deleted_events WHERE event_id = ?",
            [&id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .expect("the tombstone is in the existing table");
    assert_eq!(tombstone, (keys.public_key().to_hex(), t as i64));
}

#[tokio::test]
async fn a_kind_5_removes_the_authors_event_by_address_and_it_stays_removed() {
    let (store, path, _dir) = open().await;
    let keys = Keys::generate();
    let pubkey = keys.public_key().to_hex();
    let t = now();
    // Addressable kinds are not stored by this build yet (#195); the file may
    // hold them all the same, written by the TypeScript relay.
    let doomed = signed_by(&keys, 30_023, t - 10, &[&["d", "doomed"]]);
    let kept = signed_by(&keys, 30_023, t - 10, &[&["d", "kept"]]);
    let connection = Connection::open(&path).expect("opens");
    for event in [&doomed, &kept] {
        connection
            .execute(
                "INSERT INTO events (id, pubkey, kind, content, tags, created_at, sig, received_at) \
                 VALUES (?, ?, ?, ?, ?, ?, ?, 0)",
                rusqlite::params![
                    event.id.to_hex(),
                    pubkey,
                    30_023,
                    event.content,
                    serde_json::to_string(&event.tags).expect("tags are JSON"),
                    event.created_at.as_secs() as i64,
                    event.sig.to_string(),
                ],
            )
            .expect("a row as the TypeScript relay writes it");
    }

    let address = format!("30023:{pubkey}:doomed");
    store
        .save(&verified(&signed_by(&keys, 5, t, &[&["a", &address]])))
        .await
        .expect("saved");
    let left: BTreeSet<String> = store
        .query(Filter::new().kind(nostr::event::Kind::from(30_023)))
        .await
        .expect("query")
        .iter()
        .map(|event| event.id.to_hex())
        .collect();
    assert_eq!(left, id_set(&[&kept]));

    let deleted_at: i64 = connection
        .query_row(
            "SELECT deleted_at FROM deleted_addresses WHERE coordinate = ?",
            [&address],
            |row| row.get(0),
        )
        .expect("the tombstone is in the existing table");
    assert_eq!(deleted_at, t as i64);

    // An older version at that address is retracted; a newer one is not.
    let older = signed_by(&keys, 30_023, t - 5, &[&["d", "doomed"]]);
    assert!(
        retracted(&connection, &older),
        "an address tombstone covers what was written up to it"
    );
}

fn retracted(connection: &Connection, event: &Event) -> bool {
    let coordinate = format!("30023:{}:doomed", event.pubkey.to_hex());
    let at: i64 = connection
        .query_row(
            "SELECT deleted_at FROM deleted_addresses WHERE coordinate = ?",
            [coordinate],
            |row| row.get(0),
        )
        .expect("a tombstone");
    event.created_at.as_secs() as i64 <= at
}

#[tokio::test]
async fn a_kind_5_cannot_delete_another_authors_event_nor_stop_its_republication() {
    let (store, path, _dir) = open().await;
    let victim = Keys::generate();
    let attacker = Keys::generate();
    let t = now();
    let theirs = signed_by(&victim, 1, t - 10, &[]);
    store.save(&verified(&theirs)).await.expect("saved");

    let by_attacker = signed_by(
        &attacker,
        5,
        t,
        &[
            &["e", &theirs.id.to_hex()],
            &["a", &format!("30023:{}:mine", victim.public_key().to_hex())],
        ],
    );
    store.save(&verified(&by_attacker)).await.expect("saved");
    assert_eq!(held(&store, &victim).await, id_set(&[&theirs]));

    // The tombstone the request left names the attacker, so it binds only
    // the attacker's own copy.
    let fresh = Store::open(&path).expect("reopens");
    assert_eq!(
        fresh.save(&verified(&theirs)).await.expect("answered"),
        Saved::Duplicate
    );
    let address_tombstones: i64 = Connection::open(&path)
        .expect("opens")
        .query_row("SELECT COUNT(*) FROM deleted_addresses", [], |row| {
            row.get(0)
        })
        .expect("counts");
    assert_eq!(
        address_tombstones, 0,
        "an address in another's name is ignored"
    );
}

#[tokio::test]
async fn a_deletion_that_arrives_before_its_target_still_retracts_it() {
    let (store, _path, _dir) = open().await;
    let keys = Keys::generate();
    let t = now();
    let target = signed_by(&keys, 1, t - 10, &[]);
    store
        .save(&verified(&signed_by(
            &keys,
            5,
            t,
            &[&["e", &target.id.to_hex()]],
        )))
        .await
        .expect("saved");
    assert_eq!(
        store.save(&verified(&target)).await.expect("answered"),
        Saved::Dropped
    );
}

#[tokio::test]
async fn an_expired_event_is_left_out_of_queries_only_while_expiration_is_enforced() {
    let dir = tempdir().expect("a temp dir");
    let path = typescript_database(dir.path());
    let keys = Keys::generate();
    let t = now();
    let expired = signed_by(
        &keys,
        1,
        t - 200,
        &[&["expiration", &(t - 100).to_string()]],
    );
    let live = signed_by(
        &keys,
        1,
        t - 200,
        &[&["expiration", &(t + 3600).to_string()]],
    );
    let plain = signed_by(&keys, 1, t - 200, &[&["t", "plain"]]);

    let enforcing = Store::open(&path).expect("opens");
    for event in [&expired, &live, &plain] {
        enforcing.save(&verified(event)).await.expect("saved");
    }
    assert!(!enforcing.serves(&expired));
    assert!(enforcing.serves(&live) && enforcing.serves(&plain));
    assert_eq!(held(&enforcing, &keys).await, id_set(&[&live, &plain]));
    let limited = enforcing
        .query(Filter::new().author(keys.public_key()).limit(2))
        .await
        .expect("query");
    assert_eq!(limited.len(), 2, "a limit counts what is served");

    let lax = Store::open_with(
        &path,
        Retention {
            enforce_expiration: false,
            ..Retention::default()
        },
    )
    .expect("opens");
    assert!(lax.serves(&expired));
    assert_eq!(held(&lax, &keys).await, id_set(&[&expired, &live, &plain]));
}

#[tokio::test]
async fn the_reaper_deletes_only_what_expired_longer_ago_than_the_grace() {
    let (store, path, _dir) = open().await;
    let keys = Keys::generate();
    let t = now();
    let long_dead = signed_by(
        &keys,
        1,
        t - 9000,
        &[&["expiration", &(t - 5000).to_string()]],
    );
    let just_dead = signed_by(
        &keys,
        1,
        t - 9000,
        &[&["expiration", &(t - 100).to_string()]],
    );
    let live = signed_by(
        &keys,
        1,
        t - 9000,
        &[&["expiration", &(t + 100).to_string()]],
    );
    let plain = signed_by(&keys, 1, t - 9000, &[]);
    for event in [&long_dead, &just_dead, &live, &plain] {
        store.save(&verified(event)).await.expect("saved");
    }

    assert_eq!(store.reap_expired(1000).await.expect("reaps"), 1);
    assert_eq!(store.reap_expired(1000).await.expect("reaps"), 0);
    assert_eq!(store.reap_expired(0).await.expect("reaps"), 1);

    let rows: BTreeSet<String> = Connection::open(&path)
        .expect("opens")
        .prepare("SELECT id FROM events")
        .expect("prepares")
        .query_map([], |row| row.get(0))
        .expect("runs")
        .collect::<Result<_, _>>()
        .expect("rows");
    assert_eq!(rows, id_set(&[&live, &plain]));
}

#[tokio::test]
async fn a_blocklisted_id_is_dropped_silently_and_swept_from_the_file_at_open() {
    let dir = tempdir().expect("a temp dir");
    let path = typescript_database(dir.path());
    let keys = Keys::generate();
    let blocked = signed_by(&keys, 1, 1_700_000_000, &[&["t", "litter"]]);
    let other = signed_by(&keys, 1, 1_700_000_000, &[]);

    let before = Store::open(&path).expect("opens");
    before.save(&verified(&blocked)).await.expect("saved");
    before.save(&verified(&other)).await.expect("saved");

    let blocking = Store::open_with(
        &path,
        Retention {
            blocked_event_ids: BTreeSet::from([blocked.id.to_hex()]),
            ..Retention::default()
        },
    )
    .expect("opens");
    assert_eq!(
        held(&blocking, &keys).await,
        id_set(&[&other]),
        "swept at open"
    );
    assert_eq!(
        blocking.save(&verified(&blocked)).await.expect("answered"),
        Saved::Dropped
    );
    assert_eq!(held(&blocking, &keys).await, id_set(&[&other]));
}

#[tokio::test]
async fn a_blocked_write_is_answered_as_a_stored_one_and_stores_nothing() {
    let blocked = signed(1, 1_700_000_000, &[]);
    let running = running_with(|name| match name {
        "TOON_BLOCKED_EVENT_IDS" => Some(blocked.id.to_hex()),
        _ => None,
    })
    .await;
    let (status, body) = write(&running.relay, delivery(&blocked)).await;
    assert_eq!(status, 200);
    assert_eq!(body["eventId"], blocked.id.to_hex());
    let stored: i64 = Connection::open(&running.database)
        .expect("opens")
        .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
        .expect("counts");
    assert_eq!(stored, 0);
}

#[tokio::test]
async fn the_reaper_sweeps_at_boot_and_a_zero_interval_never_starts_it() {
    let t = now();
    let expired = signed(1, t - 9000, &[&["expiration", &(t - 5000).to_string()]]);
    let dir = tempdir().expect("a temp dir");
    let path = typescript_database(dir.path());
    Store::open(&path)
        .expect("opens")
        .save(&verified(&expired))
        .await
        .expect("saved");
    let data_dir = dir.path().to_string_lossy().into_owned();
    let config = |interval: &'static str| {
        let data_dir = data_dir.clone();
        relay::Config::from_env(move |name| match name {
            "TOON_SECRET_KEY" => Some("1".repeat(64)),
            "TOON_DATA_DIR" => Some(data_dir.clone()),
            "TOON_EXPIRATION_REAP_GRACE_SECONDS" => Some("0".to_string()),
            "TOON_EXPIRATION_REAP_INTERVAL_SECONDS" => Some(interval.to_string()),
            _ => None,
        })
        .expect("a valid configuration")
    };
    let rows = || -> i64 {
        Connection::open(&path)
            .expect("opens")
            .query_row("SELECT COUNT(*) FROM events", [], |row| row.get(0))
            .expect("counts")
    };

    let disabled = relay::Relay::open(&config("0")).expect("opens");
    assert!(disabled.spawn_reaper().is_none());
    assert_eq!(rows(), 1);

    let enabled = relay::Relay::open(&config("3600")).expect("opens");
    let reaper = enabled
        .spawn_reaper()
        .expect("an interval above zero runs it");
    for _ in 0..100 {
        if rows() == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(rows(), 0, "the boot sweep ran");
    reaper.abort();
}
