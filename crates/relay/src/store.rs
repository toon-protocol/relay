//! The event store: the SQLite file the TypeScript relay already writes, with
//! its three tables and its indexes, opened as it is. A database written by
//! either implementation must stay readable and writable by the other (#185),
//! so nothing here changes the schema and a row is written exactly as the
//! TypeScript relay writes it.
//!
//! [`Store::save`] takes a [`VerifiedEvent`] and nothing else: that is the one
//! way an event reaches the file.
//!
//! It keeps regular events, replaces replaceable and addressable ones, and
//! answers filters. Deletion (#196) and ephemeral kinds (#198) are not built,
//! so the kinds they govern are refused rather than stored under the wrong
//! rule.
//!
//! What it does not do yet, and a reader should not assume: it does not
//! consult the tombstone tables or the operator blocklist before saving, so a
//! regular event its author deleted through the TypeScript relay would be
//! admitted again; and it does not leave expired events out of a query
//! (#196). It writes `expires_at`, because the row must be the one the
//! TypeScript relay would write.

use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use nostr::event::{Event, Tags};
use nostr::filter::Filter;
use rusqlite::types::Value;
use rusqlite::{Connection, params, params_from_iter};

use crate::clock::unix_seconds;
use crate::{RelayError, VerifiedEvent};

/// The schema, statement for statement as the TypeScript relay creates it.
/// SQLite records each statement's text, so these are kept identical to
/// `tests/fixtures/typescript-schema.sql` apart from `IF NOT EXISTS`, which is
/// not recorded.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS events (
  id TEXT PRIMARY KEY,
  pubkey TEXT NOT NULL,
  kind INTEGER NOT NULL,
  content TEXT NOT NULL,
  tags TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  sig TEXT NOT NULL,
  received_at INTEGER NOT NULL,
  expires_at INTEGER
);
CREATE TABLE IF NOT EXISTS deleted_events (
  event_id TEXT PRIMARY KEY,
  pubkey TEXT NOT NULL,
  deleted_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS deleted_addresses (
  coordinate TEXT PRIMARY KEY,
  deleted_at INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_events_pubkey ON events(pubkey);
CREATE INDEX IF NOT EXISTS idx_events_kind ON events(kind);
CREATE INDEX IF NOT EXISTS idx_events_created_at ON events(created_at);
CREATE INDEX IF NOT EXISTS idx_events_pubkey_kind ON events(pubkey, kind);
CREATE INDEX IF NOT EXISTS idx_events_expires_at ON events(expires_at) WHERE expires_at IS NOT NULL;
";

/// `OR IGNORE`: a second copy of an event already held changes nothing.
const INSERT: &str = "
INSERT OR IGNORE INTO events
  (id, pubkey, kind, content, tags, created_at, sig, received_at, expires_at)
VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)";

const SELECT: &str = "SELECT id, pubkey, created_at, kind, tags, content, sig FROM events";

/// One tag condition: the event carries a tag whose name is the first
/// parameter and whose value is one of the rest. The comparison is on the
/// parsed JSON, so a `%` or `_` in a value is text and a value matches whole.
///
/// `json_valid` comes first because `json_each` raises on text that is not
/// JSON, which would fail the whole query over one bad row; the `CASE` is
/// what guarantees the order, which `AND` does not.
const HAS_TAG: &str = "CASE WHEN json_valid(events.tags) THEN EXISTS (\
     SELECT 1 FROM json_each(events.tags) AS tag \
     WHERE json_extract(tag.value, '$[0]') = ? AND json_extract(tag.value, '$[1]') IN";

/// The largest whole number the TypeScript relay reads from an `expiration`
/// tag: JavaScript's `Number.MAX_SAFE_INTEGER`.
const MAX_EXPIRATION: u64 = (1 << 53) - 1;

/// What [`Store::save`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Saved {
    /// The event was not held before and is now.
    New,
    /// The event was already held; nothing was written.
    Duplicate,
    /// A held event of the same kind, author (and `d` tag, if addressable)
    /// is newer, or as new and lower by id; nothing was written.
    Superseded,
}

/// The event store over one SQLite file. Cheap to clone: every clone is the
/// same connection.
#[derive(Debug, Clone)]
pub struct Store {
    connection: Arc<Mutex<Connection>>,
}

impl Store {
    /// Open the database at `path`, creating the file and the schema if they
    /// are not there. An existing database is opened as it is.
    pub fn open(path: &Path) -> Result<Self, RelayError> {
        let open = || {
            let connection = Connection::open(path)?;
            // The TypeScript relay's journal settings (connector#685): a
            // write is a WAL append, not two fsyncs.
            connection.pragma_update(None, "journal_mode", "WAL")?;
            connection.pragma_update(None, "synchronous", "NORMAL")?;
            connection.execute_batch(SCHEMA)?;
            // A database whose `events` table is not this one is refused now,
            // not on the first write.
            connection.prepare(INSERT)?;
            connection.prepare(SELECT)?;
            Ok(connection)
        };
        let connection = open().map_err(|source| RelayError::StoreOpen {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }

    /// Save a verified event.
    ///
    /// A replaceable or addressable kind replaces the event it supersedes in
    /// the same transaction, or is [`Saved::Superseded`]. A kind governed by a
    /// rule this store does not have (deletion, ephemeral) is
    /// [`RelayError::KindNotStoredYet`] and nothing is written.
    pub async fn save(&self, event: &VerifiedEvent) -> Result<Saved, RelayError> {
        let event = event.clone();
        self.blocking(move |connection| save(connection, event.event()))
            .await
    }

    /// The stored events matching `filter`, newest first, at most its `limit`.
    pub async fn query(&self, filter: Filter) -> Result<Vec<Event>, RelayError> {
        self.blocking(move |connection| query(connection, &filter))
            .await
    }

    /// Run `work` on the connection, off the async worker threads: SQLite
    /// calls block.
    async fn blocking<T, F>(&self, work: F) -> Result<T, RelayError>
    where
        T: Send + 'static,
        F: FnOnce(&Connection) -> Result<T, RelayError> + Send + 'static,
    {
        let connection = Arc::clone(&self.connection);
        tokio::task::spawn_blocking(move || {
            // A panic while the lock was held cannot leave a statement half
            // run (SQLite rolls it back), so the connection is still good.
            let connection = connection.lock().unwrap_or_else(PoisonError::into_inner);
            work(&connection)
        })
        .await
        .map_err(|_| RelayError::StoreStopped)?
    }
}

/// How a kind is kept. The ranges are #185's, not the protocol crate's,
/// which also calls kind 41 replaceable and classifies 10032-10099 as
/// replaceable where this relay keys them by `d` tag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rule {
    /// One row per event.
    Regular,
    /// One row per author and kind.
    Replaceable,
    /// One row per author, kind and `d` tag.
    Addressable,
    /// Not built yet: deletion (#196) and ephemeral (#198).
    Unbuilt,
}

fn rule(kind: u16) -> Rule {
    match kind {
        5 | 20_000..=29_999 => Rule::Unbuilt,
        10_032..=10_099 | 30_000..=39_999 => Rule::Addressable,
        0 | 3 | 10_000..=19_999 => Rule::Replaceable,
        _ => Rule::Regular,
    }
}

/// The event's `d` tag value: the first `d` tag's, or empty if it has none.
fn d_value(tags: &Tags) -> &str {
    tags.iter()
        .find(|tag| tag.kind() == "d")
        .map_or("", |tag| tag.content().unwrap_or(""))
}

fn save(connection: &Connection, event: &Event) -> Result<Saved, RelayError> {
    let kind = event.kind.as_u16();
    let rule = rule(kind);
    if rule == Rule::Unbuilt {
        return Err(RelayError::KindNotStoredYet { kind });
    }
    let tags = serde_json::to_string(&event.tags).map_err(RelayError::TagsNotJson)?;
    let id = event.id.to_hex();
    let pubkey = event.pubkey.to_hex();
    let created_at = event.created_at.as_secs();

    // The read and the write are one transaction. The connection is behind a
    // mutex, so no other writer in this process can read the old row between.
    let transaction = connection.unchecked_transaction()?;
    if rule != Rule::Regular {
        let mut statement = transaction.prepare_cached(
            "SELECT id, created_at, tags FROM events WHERE pubkey = ? AND kind = ?",
        )?;
        let mut rows = statement.query(params![pubkey, kind])?;
        let mut held: Vec<String> = Vec::new();
        let mut superseded = false;
        while let Some(row) = rows.next()? {
            let held_id: String = row.get(0)?;
            let held_at: i64 = row.get(1)?;
            let held_tags: String = row.get(2)?;
            if rule == Rule::Addressable && !same_address(&held_tags, d_value(&event.tags)) {
                continue;
            }
            if held_id == id {
                return Ok(Saved::Duplicate);
            }
            // The newer event wins; on a tie, the lower id.
            let held_wins = match held_at.cmp(&i64::try_from(created_at).unwrap_or(i64::MAX)) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Equal => held_id < id,
                std::cmp::Ordering::Less => false,
            };
            superseded |= held_wins;
            held.push(held_id);
        }
        drop(rows);
        drop(statement);
        if superseded {
            return Ok(Saved::Superseded);
        }
        for held_id in held {
            transaction.execute("DELETE FROM events WHERE id = ?", params![held_id])?;
        }
    }
    let inserted = transaction.execute(
        INSERT,
        params![
            id,
            pubkey,
            kind,
            event.content,
            tags,
            seconds(created_at),
            event.sig.to_string(),
            seconds(unix_seconds()),
            expiration(&event.tags).map(seconds),
        ],
    )?;
    transaction.commit()?;
    Ok(if inserted == 0 {
        Saved::Duplicate
    } else {
        Saved::New
    })
}

/// Whether a stored row's tags carry exactly `d` as their `d` tag value. Text
/// that is not tags is no address.
fn same_address(stored_tags: &str, d: &str) -> bool {
    serde_json::from_str::<Tags>(stored_tags).is_ok_and(|tags| d_value(&tags) == d)
}

/// The event's NIP-40 expiration as the TypeScript relay reads it: the value
/// of the first `expiration` tag that is a plain whole number.
fn expiration(tags: &Tags) -> Option<u64> {
    tags.iter()
        .filter(|tag| tag.kind() == "expiration")
        .filter_map(|tag| tag.content())
        .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
        .find_map(|value| value.parse::<u64>().ok().filter(|at| *at <= MAX_EXPIRATION))
}

fn query(connection: &Connection, filter: &Filter) -> Result<Vec<Event>, RelayError> {
    let mut conditions: Vec<String> = Vec::new();
    let mut parameters: Vec<Value> = Vec::new();

    let mut one_of = |column: &str, values: Vec<Value>| {
        conditions.push(format!("{column} IN ({})", placeholders(values.len())));
        parameters.extend(values);
    };
    if let Some(ids) = &filter.ids {
        one_of("id", ids.iter().map(|id| id.to_hex().into()).collect());
    }
    if let Some(authors) = &filter.authors {
        one_of(
            "pubkey",
            authors.iter().map(|key| key.to_hex().into()).collect(),
        );
    }
    if let Some(kinds) = &filter.kinds {
        one_of(
            "kind",
            kinds
                .iter()
                .map(|kind| i64::from(kind.as_u16()).into())
                .collect(),
        );
    }
    if let Some(since) = filter.since {
        conditions.push("created_at >= ?".to_string());
        parameters.push(seconds(since.as_secs()));
    }
    if let Some(until) = filter.until {
        conditions.push("created_at <= ?".to_string());
        parameters.push(seconds(until.as_secs()));
    }
    for (name, values) in &filter.generic_tags {
        conditions.push(format!(
            "{HAS_TAG} ({})) ELSE 0 END",
            placeholders(values.len())
        ));
        parameters.push(name.to_string().into());
        parameters.extend(values.iter().cloned().map(Value::from));
    }

    let mut sql = SELECT.to_string();
    if !conditions.is_empty() {
        sql.push_str(" WHERE ");
        sql.push_str(&conditions.join(" AND "));
    }
    // Newest first, and the lower id first among equals: NIP-01's order.
    sql.push_str(" ORDER BY created_at DESC, id ASC LIMIT ?");
    // SQLite reads a negative limit as none.
    parameters.push(Value::Integer(
        filter
            .limit
            .map_or(-1, |limit| i64::try_from(limit).unwrap_or(i64::MAX)),
    ));

    let mut statement = connection.prepare_cached(&sql)?;
    let mut rows = statement.query(params_from_iter(parameters))?;
    let mut events = Vec::new();
    while let Some(row) = rows.next()? {
        let id: String = row.get(0)?;
        let pubkey: String = row.get(1)?;
        let created_at: i64 = row.get(2)?;
        let kind: i64 = row.get(3)?;
        let tags: String = row.get(4)?;
        let content: String = row.get(5)?;
        let sig: String = row.get(6)?;
        let event = serde_json::from_str(&tags).and_then(|tags: serde_json::Value| {
            serde_json::from_value::<Event>(serde_json::json!({
                "id": id,
                "pubkey": pubkey,
                "created_at": created_at,
                "kind": kind,
                "tags": tags,
                "content": content,
                "sig": sig,
            }))
        });
        match event {
            Ok(event) => events.push(event),
            // One row that is not an event (written by hand, or by a relay
            // older than this schema's checks) must not fail every query
            // that reaches it. It is left where it is and not served.
            Err(error) => eprintln!("store: row {id} is not a Nostr event and is skipped: {error}"),
        }
    }
    Ok(events)
}

/// `?, ?, …` for `count` parameters. An empty list is `NULL`, which no value
/// is `IN`: a filter naming no ids matches nothing.
fn placeholders(count: usize) -> String {
    if count == 0 {
        return "NULL".to_string();
    }
    vec!["?"; count].join(", ")
}

fn seconds(timestamp: u64) -> Value {
    Value::Integer(i64::try_from(timestamp).unwrap_or(i64::MAX))
}
