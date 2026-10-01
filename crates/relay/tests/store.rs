//! The store over the SQLite file the TypeScript relay already writes: what it
//! saves, what it hands back, and that it leaves the schema as it found it.

mod common;

use common::{recorded_schema, signed, signed_by, typescript_database, verified};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::Keys;
use nostr::types::Timestamp;
use relay::{RelayError, Saved, Store};
use rusqlite::Connection;
use tempfile::tempdir;

fn ids(events: &[nostr::event::Event]) -> Vec<String> {
    events.iter().map(|event| event.id.to_hex()).collect()
}

#[tokio::test]
async fn a_regular_event_is_saved_into_a_typescript_database_without_changing_its_schema() {
    let dir = tempdir().expect("a temp dir");
    let path = typescript_database(dir.path());
    let before = recorded_schema(&path);
    let event = signed(
        1,
        1_700_000_000,
        &[&["t", "nostr"], &["e", "abc", "wss://r"]],
    );

    let store = Store::open(&path).expect("a TypeScript database opens");
    let saved = store.save(&verified(&event)).await.expect("it is saved");
    assert_eq!(saved, Saved::New);
    let found = store
        .query(Filter::new().id(event.id))
        .await
        .expect("the query runs");
    assert_eq!(found, vec![event.clone()]);
    drop(store);

    assert_eq!(recorded_schema(&path), before, "the schema is untouched");

    // The row is the one the TypeScript relay would have written: hex ids,
    // tags as compact JSON, no expiry.
    let connection = Connection::open(&path).expect("the database reopens");
    let row: (
        String,
        String,
        i64,
        String,
        String,
        i64,
        String,
        Option<i64>,
    ) = connection
        .query_row(
            "SELECT id, pubkey, kind, content, tags, created_at, sig, expires_at FROM events",
            [],
            |row| {
                Ok((
                    row.get(0)?,
                    row.get(1)?,
                    row.get(2)?,
                    row.get(3)?,
                    row.get(4)?,
                    row.get(5)?,
                    row.get(6)?,
                    row.get(7)?,
                ))
            },
        )
        .expect("exactly one event row");
    assert_eq!(
        row,
        (
            event.id.to_hex(),
            event.pubkey.to_hex(),
            1,
            "conformance".to_string(),
            r#"[["t","nostr"],["e","abc","wss://r"]]"#.to_string(),
            1_700_000_000,
            event.sig.to_string(),
            None,
        )
    );
}

#[tokio::test]
async fn a_new_data_directory_gets_the_schema_the_typescript_relay_creates() {
    let typescript = tempdir().expect("a temp dir");
    let expected = recorded_schema(&typescript_database(typescript.path()));

    let dir = tempdir().expect("a temp dir");
    let path = dir.path().join("events.db");
    drop(Store::open(&path).expect("a missing file is created"));

    assert_eq!(recorded_schema(&path), expected);
}

#[tokio::test]
async fn an_event_saved_twice_is_a_duplicate_and_is_kept_once() {
    let dir = tempdir().expect("a temp dir");
    let store = Store::open(&typescript_database(dir.path())).expect("it opens");
    let event = verified(&signed(1, 1_700_000_000, &[]));

    assert_eq!(store.save(&event).await.expect("saved"), Saved::New);
    assert_eq!(store.save(&event).await.expect("saved"), Saved::Duplicate);
    let found = store.query(Filter::new()).await.expect("the query runs");
    assert_eq!(found.len(), 1);
}

#[tokio::test]
async fn an_expiration_tag_is_copied_into_the_expires_at_column() {
    let dir = tempdir().expect("a temp dir");
    let path = typescript_database(dir.path());
    let store = Store::open(&path).expect("it opens");
    // The TypeScript relay takes the first tag that is a plain whole number.
    let event = signed(
        1,
        1_700_000_000,
        &[&["expiration", "soon"], &["expiration", "4102444800"]],
    );
    store.save(&verified(&event)).await.expect("saved");
    drop(store);

    let expires_at: Option<i64> = Connection::open(&path)
        .expect("the database reopens")
        .query_row("SELECT expires_at FROM events", [], |row| row.get(0))
        .expect("one row");
    assert_eq!(expires_at, Some(4_102_444_800));
}

#[tokio::test]
async fn kinds_whose_storage_rule_is_not_built_yet_are_refused_and_not_stored() {
    let dir = tempdir().expect("a temp dir");
    let store = Store::open(&typescript_database(dir.path())).expect("it opens");

    // Replaceable (0, 3, 10000-19999), deletion (5), ephemeral (20000-29999),
    // addressable (30000-39999).
    for kind in [
        0, 3, 5, 10_002, 10_032, 19_999, 20_000, 29_999, 30_000, 39_999,
    ] {
        let result = store
            .save(&verified(&signed(kind, 1_700_000_000, &[])))
            .await;
        assert!(
            matches!(result, Err(RelayError::KindNotStoredYet { kind: refused }) if refused == kind),
            "kind {kind}"
        );
    }
    for kind in [1, 4, 7, 44, 1_000, 7_777, 9_999, 40_000] {
        let result = store
            .save(&verified(&signed(kind, 1_700_000_000, &[])))
            .await;
        assert!(matches!(result, Ok(Saved::New)), "kind {kind}");
    }
    let found = store.query(Filter::new()).await.expect("the query runs");
    assert_eq!(found.len(), 8);
}

#[tokio::test]
async fn a_query_returns_the_newest_matches_first_up_to_the_filters_limit() {
    let dir = tempdir().expect("a temp dir");
    let store = Store::open(&typescript_database(dir.path())).expect("it opens");
    let author = Keys::generate();
    let old = signed_by(&author, 1, 1_000, &[]);
    let middle = signed_by(&author, 1, 2_000, &[]);
    let new = signed_by(&author, 7, 3_000, &[]);
    let other = signed(1, 4_000, &[]);
    for event in [&old, &middle, &new, &other] {
        store.save(&verified(event)).await.expect("saved");
    }

    let by_author = Filter::new().author(author.public_key());
    let found = store.query(by_author.clone()).await.expect("runs");
    assert_eq!(
        ids(&found),
        ids(&[new.clone(), middle.clone(), old.clone()])
    );

    let found = store.query(by_author.clone().limit(2)).await.expect("runs");
    assert_eq!(ids(&found), ids(&[new.clone(), middle.clone()]));

    let found = store
        .query(by_author.clone().kind(nostr::event::Kind::from(1)))
        .await
        .expect("runs");
    assert_eq!(ids(&found), ids(&[middle.clone(), old.clone()]));

    let window = by_author
        .since(Timestamp::from(2_000))
        .until(Timestamp::from(2_999));
    let found = store.query(window).await.expect("runs");
    assert_eq!(ids(&found), ids(&[middle]));

    let found = store.query(Filter::new().limit(0)).await.expect("runs");
    assert_eq!(found, vec![]);
}

#[tokio::test]
async fn a_tag_filter_matches_the_whole_value_and_treats_like_wildcards_as_text() {
    let dir = tempdir().expect("a temp dir");
    let store = Store::open(&typescript_database(dir.path())).expect("it opens");
    let literal = signed(1, 1_000, &[&["t", "100%"]]);
    let longer = signed(1, 2_000, &[&["t", "100 percent"]]);
    let other_key = signed(1, 3_000, &[&["r", "100%"]]);
    for event in [&literal, &longer, &other_key] {
        store.save(&verified(event)).await.expect("saved");
    }
    let t = SingleLetterTag::LOWERCASE_T;

    let found = store
        .query(Filter::new().custom_tag(t, "100%"))
        .await
        .expect("runs");
    assert_eq!(ids(&found), ids(&[literal]));

    let found = store
        .query(Filter::new().custom_tag(t, "100"))
        .await
        .expect("runs");
    assert_eq!(found, vec![], "a prefix of a value is not the value");
}
