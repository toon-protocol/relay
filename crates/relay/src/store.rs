//! The event store: the SQLite file the TypeScript relay already writes, with
//! its three tables and its indexes, opened as it is. A database written by
//! either implementation must stay readable and writable by the other (#185),
//! so nothing here changes the schema and a row is written exactly as the
//! TypeScript relay writes it.
//!
//! [`Store::save`] takes a [`VerifiedEvent`] and nothing else: that is the one
//! way an event reaches the file.
//!
//! It keeps regular events, replaces replaceable and addressable ones,
//! applies deletion requests, and answers filters. Ephemeral kinds (#198) are
//! never kept: the write side delivers them without asking the store, which
//! refuses them rather than store them under the wrong rule.
//!
//! Retention (#196) is the store's, as the TypeScript store's is:
//!
//! - A kind 5 retracts the author's own events, by id and by address, and
//!   leaves a tombstone in `deleted_events` / `deleted_addresses`, so
//!   publishing the event again does not bring it back. It is then stored
//!   like any regular event.
//! - An id on the operator's blocklist is dropped silently, and swept from
//!   the file when the store opens.
//! - With expiration enforced, a query leaves out an event whose
//!   `expires_at` has passed, and [`Store::reap_expired`] deletes the ones
//!   that expired longer ago than a grace period.

use std::collections::{BTreeSet, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, PoisonError};

use nostr::event::{Event, Tags};
use nostr::filter::Filter;
use rusqlite::types::Value;
use rusqlite::{Connection, OptionalExtension, params, params_from_iter};

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

/// The operator's retention settings, which the store enforces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retention {
    /// Leave events past their NIP-40 expiration out of every answer.
    pub enforce_expiration: bool,
    /// Event ids (64 lowercase hex characters) that are never stored.
    pub blocked_event_ids: BTreeSet<String>,
}

impl Default for Retention {
    fn default() -> Self {
        Self {
            enforce_expiration: true,
            blocked_event_ids: BTreeSet::new(),
        }
    }
}

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
    /// The event is on the operator's blocklist, or its author has retracted
    /// it: nothing was written, and the writer is told nothing of it.
    Dropped,
}

/// The event store over one SQLite file. Cheap to clone: every clone is the
/// same connection.
#[derive(Debug, Clone)]
pub struct Store {
    connection: Arc<Mutex<Connection>>,
    retention: Arc<Retention>,
}

impl Store {
    /// Open the database at `path`, creating the file and the schema if they
    /// are not there. An existing database is opened as it is.
    pub fn open(path: &Path) -> Result<Self, RelayError> {
        Self::open_with(path, Retention::default())
    }

    /// [`Store::open`] under the operator's retention settings. Blocked ids
    /// the file already holds are deleted now: refusing them on arrival
    /// helps only with events that have not arrived yet.
    pub fn open_with(path: &Path, retention: Retention) -> Result<Self, RelayError> {
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
            let mut purge = connection.prepare("DELETE FROM events WHERE id = ?")?;
            for id in &retention.blocked_event_ids {
                purge.execute([id])?;
            }
            drop(purge);
            Ok(connection)
        };
        let connection = open().map_err(|source| RelayError::StoreOpen {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
            retention: Arc::new(retention),
        })
    }

    /// Save a verified event.
    ///
    /// A replaceable or addressable kind replaces the event it supersedes in
    /// the same transaction, or is [`Saved::Superseded`]. A deletion request
    /// (kind 5) is stored and retracts what it names. An ephemeral kind is
    /// [`RelayError::KindNotStoredYet`] and nothing is written.
    pub async fn save(&self, event: &VerifiedEvent) -> Result<Saved, RelayError> {
        self.save_then(event, |_| {}).await
    }

    /// [`Store::save`], and when the event is [`Saved::New`], `published`
    /// runs with it before the store lets any query run. An event a later
    /// query finds has therefore already been through `published`. It runs
    /// on a blocking thread holding the store: it must not wait.
    pub async fn save_then(
        &self,
        event: &VerifiedEvent,
        published: impl FnOnce(&VerifiedEvent) + Send + 'static,
    ) -> Result<Saved, RelayError> {
        let event = event.clone();
        let retention = Arc::clone(&self.retention);
        self.blocking(move |connection| {
            let saved = save(connection, &retention, event.event())?;
            if saved == Saved::New {
                published(&event);
            }
            Ok(saved)
        })
        .await
    }

    /// Whether an expired event is left out of every answer.
    pub fn enforces_expiration(&self) -> bool {
        self.retention.enforce_expiration
    }

    /// Whether `event` is one this store would serve now: false only for an
    /// expired event while expiration is enforced. Live delivery asks it, so
    /// a subscriber is not sent what a query would leave out.
    pub fn serves(&self, event: &Event) -> bool {
        !self.retention.enforce_expiration
            || expiration(&event.tags).is_none_or(|at| at > unix_seconds())
    }

    /// Delete the events that expired more than `grace` seconds ago, and say
    /// how many. The grace period is the safety net for enforcement itself:
    /// serving already leaves expired events out, so what the reaper deletes
    /// is only what nothing would have served anyway.
    pub async fn reap_expired(&self, grace: u64) -> Result<usize, RelayError> {
        self.blocking(move |connection| {
            let before = seconds(unix_seconds().saturating_sub(grace));
            Ok(connection.execute(
                "DELETE FROM events WHERE expires_at IS NOT NULL AND expires_at <= ?",
                [before],
            )?)
        })
        .await
    }

    /// The stored events matching `query`, every tag key of it included,
    /// newest first, at most its filter's `limit`.
    pub async fn query(&self, query: impl Into<Query>) -> Result<Vec<Event>, RelayError> {
        let query = query.into();
        let enforce_expiration = self.retention.enforce_expiration;
        self.blocking(move |connection| run_query(connection, &query, enforce_expiration))
            .await
    }

    /// Every stored event of a kind in `kinds`, in the order the store took
    /// them in, expired or not: the record a state built from events is
    /// replayed from when the relay opens, before anything is served. The
    /// order is the file's, not the `created_at` a writer chose.
    pub(crate) fn in_arrival_order(
        &self,
        kinds: std::ops::RangeInclusive<u16>,
    ) -> Result<Vec<Event>, RelayError> {
        let connection = self
            .connection
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut statement = connection.prepare(&format!(
            "{SELECT} WHERE kind BETWEEN ? AND ? ORDER BY rowid ASC"
        ))?;
        let rows = statement.query(params![kinds.start(), kinds.end()])?;
        events_of(rows)
    }

    /// Whether an event with this id is held.
    pub(crate) async fn holds(&self, id: String) -> Result<bool, RelayError> {
        self.blocking(move |connection| {
            Ok(connection
                .query_row("SELECT 1 FROM events WHERE id = ?", [id], |_| Ok(()))
                .optional()?
                .is_some())
        })
        .await
    }

    /// Delete the events with these ids, and say how many were held.
    pub(crate) async fn remove(&self, ids: Vec<String>) -> Result<usize, RelayError> {
        self.blocking(move |connection| {
            let mut removed = 0;
            for id in ids {
                removed += connection.execute("DELETE FROM events WHERE id = ?", [id])?;
            }
            Ok(removed)
        })
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

/// A question put to the store: a `nostr` [`Filter`], which holds single-letter
/// tag keys only, and the multi-letter ones (`#ab`) it cannot carry. All of
/// them are conditions of the query, applied before the filter's `limit`.
#[derive(Debug, Clone)]
pub struct Query {
    /// What the protocol crate reads of the filter.
    pub filter: Filter,
    /// Tag name (without `#`) and the values it may have. A key with no
    /// values matches nothing.
    pub multi_letter_tags: Vec<(String, HashSet<String>)>,
}

impl From<Filter> for Query {
    fn from(filter: Filter) -> Self {
        Self {
            filter,
            multi_letter_tags: Vec::new(),
        }
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
    /// A NIP-09 deletion request: one row per event, and it retracts what it
    /// names.
    Deletion,
    /// Never stored: the write side delivers an ephemeral event and keeps
    /// nothing (#198).
    Ephemeral,
}

fn rule(kind: u16) -> Rule {
    match kind {
        5 => Rule::Deletion,
        20_000..=29_999 => Rule::Ephemeral,
        10_032..=10_099 | 30_000..=39_999 => Rule::Addressable,
        0 | 3 | 10_000..=19_999 => Rule::Replaceable,
        _ => Rule::Regular,
    }
}

fn save(
    connection: &Connection,
    retention: &Retention,
    event: &Event,
) -> Result<Saved, RelayError> {
    let kind = event.kind.as_u16();
    let rule = rule(kind);
    if rule == Rule::Ephemeral {
        return Err(RelayError::KindNotStoredYet { kind });
    }
    if retention.blocked_event_ids.contains(&event.id.to_hex()) {
        return Ok(Saved::Dropped);
    }
    let tags = serde_json::to_string(&event.tags).map_err(RelayError::TagsNotJson)?;
    let id = event.id.to_hex();
    let pubkey = event.pubkey.to_hex();
    let created_at = seconds_i64(event.created_at.as_secs());
    let address = d_tag(&event.tags);

    // The retraction, the replacement and the insert are one transaction: a
    // deletion that failed half way must not leave its tombstones without
    // its effect. The connection is behind a mutex, so no other writer in
    // this process can read the old row between.
    let transaction = connection.unchecked_transaction()?;
    if is_retracted(&transaction, event)? {
        return Ok(Saved::Dropped);
    }
    if rule == Rule::Deletion {
        apply_deletion(&transaction, event)?;
    }
    if matches!(rule, Rule::Replaceable | Rule::Addressable) {
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
            if rule == Rule::Addressable && !same_address(&held_tags, address) {
                continue;
            }
            if held_id == id {
                return Ok(Saved::Duplicate);
            }
            // The newer event wins; on a tie, the lower id.
            let held_wins = match held_at.cmp(&created_at) {
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
            created_at,
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

/// Whether a stored row's tags carry exactly `d` as their `d` tag value, read
/// as the TypeScript relay reads them: the first tag whose name is `d`, its
/// value or empty. The row is read as plain JSON, not as nostr tags, so a row
/// the TypeScript relay stored with a tag the nostr crate rejects (an empty
/// one, say) still has its address. Text that is not a JSON array is no
/// address.
fn same_address(stored_tags: &str, d: &str) -> bool {
    let Ok(tags) = serde_json::from_str::<Vec<serde_json::Value>>(stored_tags) else {
        return false;
    };
    let held = tags
        .iter()
        .filter_map(serde_json::Value::as_array)
        .find(|tag| tag.first().and_then(serde_json::Value::as_str) == Some("d"))
        .map_or("", |tag| {
            tag.get(1).and_then(serde_json::Value::as_str).unwrap_or("")
        });
    held == d
}

/// The `d` tag value an event is addressed by: the first `d` tag's, empty
/// when there is none.
fn d_tag(tags: &Tags) -> &str {
    tags.iter()
        .find(|tag| tag.kind() == "d")
        .and_then(|tag| tag.content())
        .unwrap_or("")
}

/// Whether the event's author has retracted it: its id is tombstoned under
/// the author's key, or its address is tombstoned at or after its time.
fn is_retracted(connection: &Connection, event: &Event) -> Result<bool, RelayError> {
    let by_id: Option<String> = connection
        .query_row(
            "SELECT pubkey FROM deleted_events WHERE event_id = ?",
            [event.id.to_hex()],
            |row| row.get(0),
        )
        .optional()?;
    if by_id.is_some_and(|pubkey| pubkey == event.pubkey.to_hex()) {
        return Ok(true);
    }
    let coordinate = format!(
        "{}:{}:{}",
        event.kind.as_u16(),
        event.pubkey.to_hex(),
        d_tag(&event.tags)
    );
    let deleted_at: Option<i64> = connection
        .query_row(
            "SELECT deleted_at FROM deleted_addresses WHERE coordinate = ?",
            [coordinate],
            |row| row.get(0),
        )
        .optional()?;
    Ok(deleted_at.is_some_and(|at| seconds_i64(event.created_at.as_secs()) <= at))
}

/// Retract what the kind 5 `deletion` names, and only what its own author
/// published, no later than the deletion. The tombstone for an id is written
/// whether or not the event has arrived; [`is_retracted`] checks the author
/// when it does.
fn apply_deletion(connection: &Connection, deletion: &Event) -> Result<(), RelayError> {
    let author = deletion.pubkey.to_hex();
    let at = seconds_i64(deletion.created_at.as_secs());
    let mut ids = BTreeSet::new();
    let mut addresses = BTreeSet::new();
    for tag in deletion.tags.iter() {
        let Some(value) = tag.content() else { continue };
        match tag.kind().to_string().as_str() {
            "e" if is_hex_64(value) => {
                ids.insert(value.to_string());
            }
            "a" => {
                if let Some(address) = Address::parse(value) {
                    addresses.insert(address);
                }
            }
            _ => {}
        }
    }
    for id in ids {
        connection.execute(
            "INSERT OR REPLACE INTO deleted_events (event_id, pubkey, deleted_at) VALUES (?, ?, ?)",
            params![id, author, at],
        )?;
        connection.execute(
            "DELETE FROM events WHERE id = ? AND pubkey = ? AND created_at <= ?",
            params![id, author, at],
        )?;
    }
    for address in addresses {
        // An address in somebody else's name is ignored outright.
        if address.pubkey != author {
            continue;
        }
        // A later request must not lower an earlier one's watermark.
        connection.execute(
            "INSERT INTO deleted_addresses (coordinate, deleted_at) VALUES (?, ?) \
             ON CONFLICT(coordinate) DO UPDATE SET deleted_at = MAX(deleted_at, excluded.deleted_at)",
            params![address.coordinate(), at],
        )?;
        let mut held = connection
            .prepare("SELECT id, created_at, tags FROM events WHERE pubkey = ? AND kind = ?")?;
        let mut rows = held.query(params![address.pubkey, address.kind])?;
        let mut doomed = Vec::new();
        while let Some(row) = rows.next()? {
            let id: String = row.get(0)?;
            let created_at: i64 = row.get(1)?;
            let tags: String = row.get(2)?;
            let Ok(tags) = serde_json::from_str::<Tags>(&tags) else {
                continue;
            };
            if created_at <= at && d_tag(&tags) == address.identifier {
                doomed.push(id);
            }
        }
        for id in doomed {
            connection.execute("DELETE FROM events WHERE id = ?", [id])?;
        }
    }
    Ok(())
}

/// An `a` tag's coordinate, `<kind>:<pubkey>:<identifier>`. The identifier
/// may itself hold `:`.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Address {
    kind: u16,
    pubkey: String,
    identifier: String,
}

impl Address {
    fn parse(value: &str) -> Option<Self> {
        let (kind, rest) = value.split_once(':')?;
        let (pubkey, identifier) = rest.split_once(':')?;
        if kind.is_empty() || !kind.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        Some(Self {
            // A kind past 65535 names nothing an event can have.
            kind: kind.parse().ok()?,
            pubkey: is_hex_64(pubkey).then(|| pubkey.to_string())?,
            identifier: identifier.to_string(),
        })
    }

    /// The coordinate as the tombstone table keys it.
    fn coordinate(&self) -> String {
        format!("{}:{}:{}", self.kind, self.pubkey, self.identifier)
    }
}

fn is_hex_64(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
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

fn run_query(
    connection: &Connection,
    query: &Query,
    enforce_expiration: bool,
) -> Result<Vec<Event>, RelayError> {
    let filter = &query.filter;
    let mut conditions: Vec<String> = Vec::new();
    let mut parameters: Vec<Value> = Vec::new();

    // A condition, not a post-filter: the filter's `limit` counts only the
    // events actually served.
    if enforce_expiration {
        conditions.push("(expires_at IS NULL OR expires_at > ?)".to_string());
        parameters.push(seconds(unix_seconds()));
    }

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
    let mut has_tag = |name: String, values: Vec<String>| {
        conditions.push(format!(
            "{HAS_TAG} ({})) ELSE 0 END",
            placeholders(values.len())
        ));
        parameters.push(name.into());
        parameters.extend(values.into_iter().map(Value::from));
    };
    for (name, values) in &filter.generic_tags {
        has_tag(name.to_string(), values.iter().cloned().collect());
    }
    // The protocol crate's filter holds single-letter keys only; a longer
    // one is a condition of the same kind, so `limit` counts what it admits.
    for (name, values) in &query.multi_letter_tags {
        has_tag(name.clone(), values.iter().cloned().collect());
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
    let rows = statement.query(params_from_iter(parameters))?;
    events_of(rows)
}

/// The events of rows selected with [`SELECT`]'s columns.
fn events_of(mut rows: rusqlite::Rows<'_>) -> Result<Vec<Event>, RelayError> {
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

fn seconds_i64(timestamp: u64) -> i64 {
    i64::try_from(timestamp).unwrap_or(i64::MAX)
}

fn seconds(timestamp: u64) -> Value {
    Value::Integer(seconds_i64(timestamp))
}
