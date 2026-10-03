//! The store over the SQLite file the TypeScript relay already writes: what it
//! saves, what it hands back, and that it leaves the schema as it found it.

mod common;

use common::{recorded_schema, signed, signed_by, typescript_database, verified};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::Keys;
use nostr::types::Timestamp;
use relay::{Query, RelayError, Saved, Store};
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

    // Ephemeral (20000-29999).
    for kind in [20_000, 29_999] {
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

async fn held(store: &Store, author: &Keys, kind: u16) -> Vec<String> {
    let filter = Filter::new()
        .author(author.public_key())
        .kind(nostr::event::Kind::from(kind));
    let mut found = ids(&store.query(filter).await.expect("runs"));
    found.sort();
    found
}

fn sorted(events: &[&nostr::event::Event]) -> Vec<String> {
    let mut ids: Vec<String> = events.iter().map(|event| event.id.to_hex()).collect();
    ids.sort();
    ids
}

#[tokio::test]
async fn a_replaceable_kind_keeps_the_newest_event_per_author_and_kind() {
    let dir = tempdir().expect("a temp dir");
    let store = Store::open(&typescript_database(dir.path())).expect("it opens");
    for kind in [0, 3, 10_000, 10_002, 10_031, 10_100, 19_999] {
        let author = Keys::generate();
        let old = signed_by(&author, kind, 1_000, &[]);
        let newer = signed_by(&author, kind, 2_000, &[]);
        let newest = signed_by(&author, kind, 3_000, &[]);
        assert_eq!(store.save(&verified(&newer)).await.unwrap(), Saved::New);
        assert_eq!(
            store.save(&verified(&old)).await.unwrap(),
            Saved::Superseded
        );
        assert_eq!(held(&store, &author, kind).await, sorted(&[&newer]));
        assert_eq!(store.save(&verified(&newest)).await.unwrap(), Saved::New);
        assert_eq!(held(&store, &author, kind).await, sorted(&[&newest]));
        assert_eq!(
            store.save(&verified(&newest)).await.unwrap(),
            Saved::Duplicate
        );
    }
}

#[tokio::test]
async fn a_tie_on_created_at_keeps_the_lower_id_whichever_arrived_first() {
    let dir = tempdir().expect("a temp dir");
    let store = Store::open(&typescript_database(dir.path())).expect("it opens");
    for higher_first in [true, false] {
        let author = Keys::generate();
        let a = signed_by(&author, 10_002, 1_000, &[&["t", "a"]]);
        let b = signed_by(&author, 10_002, 1_000, &[&["t", "b"]]);
        let (lower, higher) = if a.id < b.id { (a, b) } else { (b, a) };
        let order = if higher_first {
            [&higher, &lower]
        } else {
            [&lower, &higher]
        };
        for event in order {
            store.save(&verified(event)).await.expect("saved");
        }
        assert_eq!(held(&store, &author, 10_002).await, sorted(&[&lower]));
    }
}

#[tokio::test]
async fn rows_a_replaceable_kind_already_accumulated_are_left_until_a_newer_event_replaces_them() {
    let dir = tempdir().expect("a temp dir");
    let path = typescript_database(dir.path());
    let author = Keys::generate();
    let one = signed_by(&author, 0, 1_000, &[]);
    let two = signed_by(&author, 0, 2_000, &[]);
    let connection = Connection::open(&path).expect("the database opens");
    for event in [&one, &two] {
        connection
            .execute(
                "INSERT INTO events VALUES (?, ?, 0, '', '[]', ?, ?, 1, NULL)",
                rusqlite::params![
                    event.id.to_hex(),
                    event.pubkey.to_hex(),
                    i64::try_from(event.created_at.as_secs()).unwrap(),
                    event.sig.to_string()
                ],
            )
            .expect("a row");
    }
    drop(connection);

    let store = Store::open(&path).expect("opening does not clean up");
    assert_eq!(held(&store, &author, 0).await, sorted(&[&one, &two]));

    let newest = signed_by(&author, 0, 3_000, &[]);
    store.save(&verified(&newest)).await.expect("saved");
    assert_eq!(held(&store, &author, 0).await, sorted(&[&newest]));
}

#[tokio::test]
async fn an_addressable_kind_keeps_the_newest_event_per_author_kind_and_d_tag() {
    let dir = tempdir().expect("a temp dir");
    let store = Store::open(&typescript_database(dir.path())).expect("it opens");
    for kind in [10_032, 10_050, 10_099, 30_000, 30_023, 39_999] {
        let author = Keys::generate();
        let a_old = signed_by(&author, kind, 1_000, &[&["d", "a"]]);
        let a_new = signed_by(&author, kind, 2_000, &[&["d", "a"]]);
        let b = signed_by(&author, kind, 500, &[&["d", "b"]]);
        store.save(&verified(&a_new)).await.expect("saved");
        assert_eq!(
            store.save(&verified(&a_old)).await.unwrap(),
            Saved::Superseded
        );
        store.save(&verified(&b)).await.expect("saved");
        assert_eq!(held(&store, &author, kind).await, sorted(&[&a_new, &b]));
    }
}

#[tokio::test]
async fn a_held_row_whose_tags_the_nostr_crate_rejects_is_still_replaced_by_its_address() {
    let dir = tempdir().expect("a temp dir");
    let path = typescript_database(dir.path());
    let author = Keys::generate();
    let connection = Connection::open(&path).expect("the database opens");
    connection
        .execute(
            "INSERT INTO events VALUES (?, ?, 30023, '', '[[],[\"d\",\"a\"]]', 1000, ?, 1, NULL)",
            rusqlite::params![
                "ab".repeat(32),
                author.public_key().to_hex(),
                "cd".repeat(64)
            ],
        )
        .expect("a row");
    drop(connection);

    let store = Store::open(&path).expect("it opens");
    let newer = signed_by(&author, 30_023, 2_000, &[&["d", "a"]]);
    store.save(&verified(&newer)).await.expect("saved");
    let rows: i64 = Connection::open(&path)
        .expect("the database opens")
        .query_row(
            "SELECT COUNT(*) FROM events WHERE kind = 30023",
            [],
            |row| row.get(0),
        )
        .expect("counted");
    assert_eq!(rows, 1);
}

#[tokio::test]
async fn a_missing_d_tag_is_the_empty_d_tag() {
    let dir = tempdir().expect("a temp dir");
    let store = Store::open(&typescript_database(dir.path())).expect("it opens");
    let author = Keys::generate();
    let bare = signed_by(&author, 30_023, 1_000, &[]);
    let empty = signed_by(&author, 30_023, 2_000, &[&["d", ""]]);
    store.save(&verified(&bare)).await.expect("saved");
    store.save(&verified(&empty)).await.expect("saved");
    assert_eq!(held(&store, &author, 30_023).await, sorted(&[&empty]));
}

#[tokio::test]
async fn d_tags_are_compared_exactly_so_wildcards_and_case_are_text() {
    let dir = tempdir().expect("a temp dir");
    let store = Store::open(&typescript_database(dir.path())).expect("it opens");
    let author = Keys::generate();
    let plain = signed_by(&author, 30_023, 1_000, &[&["d", "abc"]]);
    let underscore = signed_by(&author, 30_023, 2_000, &[&["d", "a_c"]]);
    let percent = signed_by(&author, 30_023, 3_000, &[&["d", "a%"]]);
    let lower = signed_by(&author, 30_023, 4_000, &[&["d", "foo"]]);
    let upper = signed_by(&author, 30_023, 5_000, &[&["d", "FOO"]]);
    for event in [&plain, &underscore, &percent, &lower, &upper] {
        store.save(&verified(event)).await.expect("saved");
    }
    assert_eq!(
        held(&store, &author, 30_023).await,
        sorted(&[&plain, &underscore, &percent, &lower, &upper])
    );

    let underscore_new = signed_by(&author, 30_023, 6_000, &[&["d", "a_c"]]);
    store.save(&verified(&underscore_new)).await.expect("saved");
    assert_eq!(
        held(&store, &author, 30_023).await,
        sorted(&[&plain, &underscore_new, &percent, &lower, &upper])
    );
}

#[tokio::test]
async fn limit_applies_to_each_filter_on_its_own() {
    let dir = tempdir().expect("a temp dir");
    let store = Store::open(&typescript_database(dir.path())).expect("it opens");
    for at in 1..=3 {
        store
            .save(&verified(&signed(1, at, &[])))
            .await
            .expect("saved");
        store
            .save(&verified(&signed(7, at, &[])))
            .await
            .expect("saved");
    }
    for kind in [1, 7] {
        let filter = Filter::new().kind(nostr::event::Kind::from(kind)).limit(2);
        assert_eq!(store.query(filter).await.expect("runs").len(), 2);
    }
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

#[tokio::test]
async fn a_row_whose_tags_are_not_json_is_not_served_and_fails_no_query() {
    let dir = tempdir().expect("a temp dir");
    let path = typescript_database(dir.path());
    Connection::open(&path)
        .expect("the database opens")
        .execute(
            "INSERT INTO events VALUES ('aa', 'bb', 1, 'by hand', 'not json', 1, 'cc', 1, NULL)",
            [],
        )
        .expect("a hand-written row");
    let store = Store::open(&path).expect("it opens");
    let event = signed(1, 1_000, &[&["t", "kept"]]);
    store.save(&verified(&event)).await.expect("saved");

    let found = store.query(Filter::new()).await.expect("runs");
    assert_eq!(ids(&found), vec![event.id.to_hex()]);

    let tagged = Filter::new().custom_tag(SingleLetterTag::LOWERCASE_T, "kept");
    let found = store.query(tagged).await.expect("a tag filter runs too");
    assert_eq!(ids(&found), vec![event.id.to_hex()]);
}

#[tokio::test]
async fn a_multi_letter_tag_key_is_a_condition_of_the_query_before_the_limit() {
    let dir = tempdir().expect("a temp dir");
    let store = Store::open(&typescript_database(dir.path())).expect("it opens");
    let tagged = signed(1, 1_000, &[&["ab", "x"]]);
    let other_value = signed(1, 1_500, &[&["ab", "y"], &["cd", "z"]]);
    let both = signed(1, 1_200, &[&["ab", "x"], &["cd", "z"], &["t", "k"]]);
    let newer = signed(1, 2_000, &[]);
    for event in [&tagged, &other_value, &both, &newer] {
        store.save(&verified(event)).await.expect("saved");
    }
    let keys = |pairs: &[(&str, &[&str])]| {
        pairs
            .iter()
            .map(|(name, values)| {
                (
                    name.to_string(),
                    values.iter().map(|v| v.to_string()).collect(),
                )
            })
            .collect()
    };
    let ask = |filter: Filter, pairs: &[(&str, &[&str])]| Query {
        filter,
        multi_letter_tags: keys(pairs),
        wrap_recipients: None,
    };

    let kind_one = Filter::new().kind(nostr::event::Kind::from(1));
    let found = store
        .query(ask(kind_one.clone().limit(1), &[("ab", &["x"])]))
        .await
        .expect("runs");
    assert_eq!(
        ids(&found),
        ids(std::slice::from_ref(&both)),
        "the newest that has it"
    );

    let found = store
        .query(ask(kind_one.clone(), &[("ab", &["x"]), ("cd", &["z"])]))
        .await
        .expect("runs");
    assert_eq!(
        ids(&found),
        ids(std::slice::from_ref(&both)),
        "every key is required"
    );

    let with_single = kind_one
        .clone()
        .custom_tag(SingleLetterTag::LOWERCASE_T, "k");
    let found = store
        .query(ask(with_single, &[("ab", &["x"])]))
        .await
        .expect("runs");
    assert_eq!(
        ids(&found),
        ids(std::slice::from_ref(&both)),
        "single-letter keys combine"
    );

    let found = store
        .query(ask(kind_one.clone(), &[("ab", &[])]))
        .await
        .expect("runs");
    assert!(found.is_empty(), "a key with no values matches nothing");

    let found = store
        .query(ask(kind_one, &[("ab", &["x", "y"])]))
        .await
        .expect("runs");
    assert_eq!(found.len(), 3);
}

#[tokio::test]
async fn a_newer_event_without_the_multi_letter_key_does_not_hide_the_one_with_it() {
    let dir = tempdir().expect("a temp dir");
    let store = Store::open(&typescript_database(dir.path())).expect("it opens");
    let tagged = signed(1, 1_000, &[&["ab", "x"]]);
    let newer = signed(1, 2_000, &[]);
    for event in [&tagged, &newer] {
        store.save(&verified(event)).await.expect("saved");
    }
    let query = Query {
        filter: Filter::new().kind(nostr::event::Kind::from(1)).limit(1),
        multi_letter_tags: vec![("ab".to_string(), ["x".to_string()].into())],
        wrap_recipients: None,
    };
    let found = store.query(query).await.expect("runs");
    assert_eq!(ids(&found), ids(std::slice::from_ref(&tagged)));
}
