//! The paid live feed's books (#215): one balance per subscriber key, the
//! filter that balance pays for, and which connections are reading on it.
//!
//! The relay holds the balance and nothing proves it to anyone else, so the
//! books are the relay's own and kept in its SQLite file, in a table that is
//! only ever added: `feed_subscriptions`. The TypeScript image never reads it
//! and opens the file as before. The table is created only when the relay
//! sells its feed, so a relay that does not leaves the file exactly as the
//! TypeScript relay wrote it.
//!
//! ## What is kept where
//!
//! The balances are held in memory, and that is the book: a debit is made
//! when an event is accepted, on the write path, and a database round trip
//! there would put a disk in every paid write. Every change is also queued,
//! in order, to one writer thread that keeps the table equal to the memory.
//! A credit waits for its row to be written before it is answered, because a
//! subscriber has paid for it; a debit does not, so a crash can forget the
//! last few debits and give those events away, and never the other way.
//!
//! ## Who is charged for what
//!
//! The ledger also knows each connection's open paid `REQ`s, registered when
//! the `REQ` arrives. [`Ledger::charge`] runs when an event is accepted, once
//! per event, and decides there, in the order the relay accepts events, which
//! subscribers pay for it: those whose subscription filter matches and who
//! hold an open `REQ` the event matches, on any of their connections. Each is
//! debited exactly one broadcast price however many `REQ`s or connections
//! the event is then sent on. A subscriber whose balance no longer covers
//! another event is named as exhausted in the same step, and its `REQ`s are
//! forgotten here at once, so the next event cannot be charged to it.

use std::collections::HashMap;
use std::path::Path;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use nostr::event::Event;
use nostr::key::PublicKey;
use rusqlite::{Connection, params};
use serde_json::Value;
use tokio::sync::oneshot;

use crate::RelayError;
use crate::clock::unix_seconds;
use crate::session::Wanted;

/// The largest balance: what survives a JSON number in every client.
const MAX_BALANCE: u64 = (1 << 53) - 1;

/// Added to the file, never changed: the TypeScript relay does not know it.
const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS feed_subscriptions (
  pubkey TEXT PRIMARY KEY,
  balance INTEGER NOT NULL,
  filter TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);
";

/// The most rows the writer puts in one transaction.
const BATCH: usize = 256;

/// A subscription as it stands.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Snapshot {
    pub(crate) pubkey: PublicKey,
    pub(crate) balance: u64,
    /// The filter as the subscriber sent it.
    pub(crate) filter: Value,
}

/// Why a credit was not made.
#[derive(Debug)]
pub(crate) enum CreditError {
    /// The key has no subscription, and a first payment names a filter.
    FilterRequired,
    /// The credit is in memory but its row could not be written.
    NotKept(String),
}

/// What accepting one event cost, and whom.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Charged {
    /// The subscribers debited for the event: it is sent to their `REQ`s.
    pub(crate) paid: Vec<PublicKey>,
    /// Those of them left without enough for another.
    pub(crate) exhausted: Vec<PublicKey>,
}

/// One connection, as the ledger knows it.
pub(crate) type ConnectionId = u64;

#[derive(Debug)]
struct Entry {
    balance: u64,
    filter: Value,
    wanted: Wanted,
}

impl Entry {
    fn snapshot(&self, pubkey: PublicKey) -> Snapshot {
        Snapshot {
            pubkey,
            balance: self.balance,
            filter: self.filter.clone(),
        }
    }
}

#[derive(Debug)]
struct Reading {
    key: PublicKey,
    requests: HashMap<String, Arc<[Wanted]>>,
}

#[derive(Debug, Default)]
struct State {
    subscriptions: HashMap<PublicKey, Entry>,
    connections: HashMap<ConnectionId, Reading>,
    next_connection: ConnectionId,
}

struct Row {
    pubkey: String,
    balance: u64,
    filter: String,
    kept: Option<oneshot::Sender<Result<(), String>>>,
}

struct Books {
    broadcast_price: u64,
    state: Mutex<State>,
    rows: Sender<Row>,
}

/// The books of a relay that sells its feed. A relay whose feed is free has
/// none. Cheap to clone; every clone is the same books.
#[derive(Clone)]
pub(crate) struct Ledger(Arc<Books>);

impl std::fmt::Debug for Ledger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Ledger")
            .field("broadcast_price", &self.0.broadcast_price)
            .finish_non_exhaustive()
    }
}

impl Ledger {
    /// Open the books in the database at `path`, adding their table if it is
    /// not there, and read every subscription back.
    pub(crate) fn open(path: &Path, broadcast_price: u64) -> Result<Self, RelayError> {
        let open = || {
            let connection = Connection::open(path)?;
            connection.pragma_update(None, "journal_mode", "WAL")?;
            connection.pragma_update(None, "synchronous", "NORMAL")?;
            connection.execute_batch(SCHEMA)?;
            Ok(connection)
        };
        let connection = open().map_err(|source| RelayError::StoreOpen {
            path: path.to_path_buf(),
            source,
        })?;
        let state = read_all(&connection).map_err(|source| RelayError::StoreOpen {
            path: path.to_path_buf(),
            source,
        })??;
        let (rows, queued) = channel();
        std::thread::Builder::new()
            .name("ledger-writer".to_string())
            .spawn(move || write_rows(connection, &queued))
            .map_err(RelayError::LedgerWriterNotStarted)?;
        Ok(Self(Arc::new(Books {
            broadcast_price,
            state: Mutex::new(state),
            rows,
        })))
    }

    /// What one broadcast event costs.
    pub(crate) fn broadcast_price(&self) -> u64 {
        self.0.broadcast_price
    }

    fn books(&self) -> (&Books, MutexGuard<'_, State>) {
        let books = &*self.0;
        let state = books.state.lock().unwrap_or_else(PoisonError::into_inner);
        (books, state)
    }

    /// Credit `amount` to `key`, opening its subscription with `filter` if it
    /// has none, and replacing the filter when one is given. Answered once
    /// the new balance is on disk, with what the credit added: `amount`, less
    /// whatever [`MAX_BALANCE`] left no room for. A credit whose row cannot
    /// be written is taken back, filter and all: the request is refused, and
    /// the draft forbids crediting a request the relay refuses.
    pub(crate) async fn credit(
        &self,
        key: PublicKey,
        amount: u64,
        filter: Option<(Value, Wanted)>,
    ) -> Result<(Snapshot, u64), CreditError> {
        let (snapshot, added, replaced, kept) = {
            let (books, mut state) = self.books();
            let (entry, replaced) = match (state.subscriptions.get_mut(&key), filter) {
                (None, None) => return Err(CreditError::FilterRequired),
                (None, Some((filter, wanted))) => (
                    state.subscriptions.entry(key).or_insert(Entry {
                        balance: 0,
                        filter,
                        wanted,
                    }),
                    None,
                ),
                (Some(entry), filter) => {
                    let replaced = filter.map(|(filter, wanted)| {
                        (
                            std::mem::replace(&mut entry.filter, filter),
                            std::mem::replace(&mut entry.wanted, wanted),
                        )
                    });
                    (entry, replaced)
                }
            };
            let before = entry.balance;
            entry.balance = entry.balance.saturating_add(amount).min(MAX_BALANCE);
            let snapshot = entry.snapshot(key);
            let (kept, wait) = oneshot::channel();
            queue(books, &snapshot, Some(kept));
            (snapshot, entry.balance - before, replaced, wait)
        };
        let reason = match kept.await {
            Ok(Ok(())) => return Ok((snapshot, added)),
            Ok(Err(reason)) => reason,
            Err(_) => "the ledger writer stopped".to_string(),
        };
        self.take_back(key, added, &snapshot.filter, replaced);
        Err(CreditError::NotKept(reason))
    }

    /// Undo a credit whose row was not written: take `added` back off `key`'s
    /// balance, and put back the filter it replaced unless a later credit has
    /// replaced it since. The row is queued again, so the table follows.
    fn take_back(
        &self,
        key: PublicKey,
        added: u64,
        filter: &Value,
        replaced: Option<(Value, Wanted)>,
    ) {
        let (books, mut state) = self.books();
        let Some(entry) = state.subscriptions.get_mut(&key) else {
            return;
        };
        entry.balance = entry.balance.saturating_sub(added);
        if let Some((previous, wanted)) = replaced
            && entry.filter == *filter
        {
            entry.filter = previous;
            entry.wanted = wanted;
        }
        let snapshot = entry.snapshot(key);
        queue(books, &snapshot, None);
    }

    /// The subscription of `key`, if it has one.
    pub(crate) fn subscription(&self, key: &PublicKey) -> Option<Snapshot> {
        let (_, state) = self.books();
        state
            .subscriptions
            .get(key)
            .map(|entry| entry.snapshot(*key))
    }

    /// Every subscription, by key.
    pub(crate) fn subscriptions(&self) -> Vec<Snapshot> {
        let (_, state) = self.books();
        let mut all: Vec<_> = state
            .subscriptions
            .iter()
            .map(|(key, entry)| entry.snapshot(*key))
            .collect();
        all.sort_by_key(|snapshot| snapshot.pubkey.to_hex());
        all
    }

    /// Whether `key` holds a subscription that is not exhausted.
    pub(crate) fn holds(&self, key: &PublicKey) -> bool {
        let (books, state) = self.books();
        state
            .subscriptions
            .get(key)
            .is_some_and(|entry| entry.balance >= books.broadcast_price)
    }

    /// A connection has opened.
    pub(crate) fn connect(&self) -> ConnectionId {
        let (_, mut state) = self.books();
        state.next_connection += 1;
        state.next_connection
    }

    /// `connection`, which holds `key`'s subscription, has an open `REQ`
    /// `id` asking for `filters`. A connection that held another key's
    /// subscription gives its `REQ`s up.
    pub(crate) fn open_request(
        &self,
        connection: ConnectionId,
        key: PublicKey,
        id: &str,
        filters: Arc<[Wanted]>,
    ) {
        let (_, mut state) = self.books();
        let reading = state.connections.entry(connection).or_insert(Reading {
            key,
            requests: HashMap::new(),
        });
        if reading.key != key {
            reading.key = key;
            reading.requests.clear();
        }
        reading.requests.insert(id.to_string(), filters);
    }

    /// `connection`'s `REQ` `id` is over.
    pub(crate) fn close_request(&self, connection: ConnectionId, id: &str) {
        let (_, mut state) = self.books();
        if let Some(reading) = state.connections.get_mut(&connection) {
            reading.requests.remove(id);
        }
    }

    /// Every `REQ` of `connection` is over, or the connection is.
    pub(crate) fn disconnect(&self, connection: ConnectionId) {
        self.books().1.connections.remove(&connection);
    }

    /// Decide who pays for `event`, which the relay has just accepted, and
    /// debit them. See the module's account of who is charged.
    pub(crate) fn charge(&self, event: &Event) -> Charged {
        let (books, mut state) = self.books();
        let mut asking: Vec<PublicKey> = state
            .connections
            .values()
            .filter(|reading| {
                reading
                    .requests
                    .values()
                    .any(|filters| filters.iter().any(|filter| filter.matches(event)))
            })
            .map(|reading| reading.key)
            .collect();
        asking.sort();
        asking.dedup();

        let mut charged = Charged::default();
        for key in asking {
            let State {
                subscriptions,
                connections,
                ..
            } = &mut *state;
            let Some(entry) = subscriptions.get_mut(&key) else {
                continue;
            };
            if entry.balance < books.broadcast_price || !entry.wanted.matches(event) {
                continue;
            }
            entry.balance -= books.broadcast_price;
            charged.paid.push(key);
            queue(books, &entry.snapshot(key), None);
            if entry.balance < books.broadcast_price {
                charged.exhausted.push(key);
                for reading in connections.values_mut().filter(|r| r.key == key) {
                    reading.requests.clear();
                }
            }
        }
        charged
    }
}

/// Put `snapshot`'s row in the writer's queue. Called with the state locked,
/// so rows are queued in the order the books changed.
fn queue(books: &Books, snapshot: &Snapshot, kept: Option<oneshot::Sender<Result<(), String>>>) {
    let row = Row {
        pubkey: snapshot.pubkey.to_hex(),
        balance: snapshot.balance,
        filter: snapshot.filter.to_string(),
        kept,
    };
    if let Err(failed) = books.rows.send(row) {
        // The writer is gone: say so to whoever waits, and log the rest.
        match failed.0.kept {
            Some(kept) => {
                let _ = kept.send(Err("the ledger writer stopped".to_string()));
            }
            None => eprintln!("ledger: a debit could not be kept: the writer stopped"),
        }
    }
}

/// The subscriptions in the table, or why a row could not be read. The outer
/// error is SQLite's; the inner is a row this build cannot read, which stops
/// the relay rather than forget a balance.
fn read_all(connection: &Connection) -> Result<Result<State, RelayError>, rusqlite::Error> {
    let mut statement =
        connection.prepare("SELECT pubkey, balance, filter FROM feed_subscriptions")?;
    let rows = statement.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut state = State::default();
    for row in rows {
        let (pubkey, balance, filter) = row?;
        let unreadable = || RelayError::LedgerRowUnreadable {
            pubkey: pubkey.clone(),
        };
        let Ok(key) = PublicKey::from_hex(&pubkey) else {
            return Ok(Err(unreadable()));
        };
        let Ok(filter) = serde_json::from_str::<Value>(&filter) else {
            return Ok(Err(unreadable()));
        };
        let Some(wanted) = Wanted::read(filter.clone()) else {
            return Ok(Err(unreadable()));
        };
        let Ok(balance) = u64::try_from(balance) else {
            return Ok(Err(unreadable()));
        };
        state.subscriptions.insert(
            key,
            Entry {
                balance,
                filter,
                wanted,
            },
        );
    }
    Ok(Ok(state))
}

/// The writer thread: until every sender is gone, take what is queued, write
/// it in one transaction and say how that went to whoever waits.
fn write_rows(mut connection: Connection, queued: &Receiver<Row>) {
    while let Ok(first) = queued.recv() {
        let mut batch = vec![first];
        while batch.len() < BATCH {
            match queued.try_recv() {
                Ok(row) => batch.push(row),
                Err(_) => break,
            }
        }
        let outcome = write_batch(&mut connection, &batch).map_err(|error| error.to_string());
        if let Err(reason) = &outcome {
            eprintln!(
                "ledger: {} row(s) could not be written: {reason}",
                batch.len()
            );
        }
        for row in batch {
            if let Some(kept) = row.kept {
                let _ = kept.send(outcome.clone());
            }
        }
    }
}

fn write_batch(connection: &mut Connection, batch: &[Row]) -> Result<(), rusqlite::Error> {
    let now = i64::try_from(unix_seconds()).unwrap_or(i64::MAX);
    let transaction = connection.transaction()?;
    {
        let mut upsert = transaction.prepare_cached(
            "INSERT INTO feed_subscriptions (pubkey, balance, filter, updated_at)
             VALUES (?, ?, ?, ?)
             ON CONFLICT(pubkey) DO UPDATE SET
               balance = excluded.balance,
               filter = excluded.filter,
               updated_at = excluded.updated_at",
        )?;
        for row in batch {
            let balance = i64::try_from(row.balance).unwrap_or(i64::MAX);
            upsert.execute(params![row.pubkey, balance, row.filter, now])?;
        }
    }
    transaction.commit()
}

#[cfg(test)]
mod tests {
    use nostr::key::Keys;
    use serde_json::json;

    use super::*;

    fn filter(value: Value) -> Option<(Value, Wanted)> {
        let wanted = Wanted::read(value.clone()).expect("a NIP-01 filter");
        Some((value, wanted))
    }

    #[tokio::test]
    async fn a_credit_answers_what_it_added_which_the_largest_balance_can_cut_short() {
        let data = tempfile::tempdir().expect("a temp dir");
        let ledger = Ledger::open(&data.path().join("events.db"), 10).expect("the books");
        let key = Keys::generate().public_key();

        let (snapshot, added) = ledger
            .credit(key, MAX_BALANCE - 5, filter(json!({ "kinds": [1] })))
            .await
            .expect("a credit");
        assert_eq!(
            (snapshot.balance, added),
            (MAX_BALANCE - 5, MAX_BALANCE - 5)
        );

        let (snapshot, added) = ledger.credit(key, 1000, None).await.expect("a credit");
        assert_eq!((snapshot.balance, added), (MAX_BALANCE, 5));
    }

    #[tokio::test]
    async fn a_credit_whose_row_is_not_written_is_taken_back_filter_and_all() {
        let data = tempfile::tempdir().expect("a temp dir");
        let path = data.path().join("events.db");
        let ledger = Ledger::open(&path, 10).expect("the books");
        let key = Keys::generate().public_key();
        ledger
            .credit(key, 100, filter(json!({ "kinds": [1] })))
            .await
            .expect("a credit");

        // The table goes, so the writer's next row fails.
        Connection::open(&path)
            .expect("the file")
            .execute_batch("DROP TABLE feed_subscriptions")
            .expect("dropped");
        let refused = ledger
            .credit(key, 50, filter(json!({ "kinds": [7] })))
            .await;
        assert!(matches!(refused, Err(CreditError::NotKept(_))));

        let standing = ledger.subscription(&key).expect("still subscribed");
        assert_eq!(standing.balance, 100);
        assert_eq!(standing.filter, json!({ "kinds": [1] }));
    }
}
