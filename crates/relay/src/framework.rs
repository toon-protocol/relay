//! The adapter over the framework: `nostr-sdk`'s `local_relay`, which parses
//! NIP-01 messages and answers each `REQ` from the store.
//!
//! The framework's API is declared alpha, so this is the only module that
//! imports it (`nostr_sdk`, and `nostr_database` for the trait it reads
//! through). The rest of the relay sees [`ReadSide`] and nothing of the
//! framework's, so a breaking upgrade, or replacing it with the relay's own
//! subscription handling, is a change to this file.
//!
//! What the framework is given, and what it is not:
//!
//! - It never binds a listener. The relay accepts the WebSocket upgrade on
//!   its own HTTP server and hands the upgraded stream to [`ReadSide::serve`].
//! - It reads the store and cannot write it. The database it is given refuses
//!   every write, so the only way into the store stays [`Store::save`], which
//!   takes a verified event.
//! - It never hears an `EVENT` from a client: the gate (`gate.rs`) refuses
//!   each one, in words that name the Write Edge once the relay knows it,
//!   because writes are paid and arrive on the write port. Its write policy
//!   refuses anyway, should one ever reach it.
//! - It speaks to a connection through the gate, which also holds the limits
//!   the framework would answer differently and the connection cap.
//! - It is never told of a new event. It would deliver one to each
//!   subscription through its own WebSocket endpoint and the pipe to the
//!   gate; the gate delivers live events itself, from [`ReadSide::deliver`]
//!   (#232), and the framework's subscriptions are only ever answered from
//!   the store.
//! - `AUTH` is neither required nor advertised: NIP-42 is left off.

use std::collections::BTreeSet;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use nostr::event::{Event, EventId};
use nostr::filter::Filter;
use nostr::message::MachineReadablePrefix;
use nostr_database::error::Error as DatabaseError;
use nostr_database::{
    DatabaseEventStatus, Features, NostrDatabase, RejectedReason, SaveEventStatus,
};
use nostr_sdk::local_relay::{LocalRelay, WritePolicy, WritePolicyResult};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Semaphore;

use crate::connector::EdgeSlot;
use crate::{Carriage, RelayError, Store, VerifiedEvent, gate};

type Answer<'a, T> = Pin<Box<dyn Future<Output = Result<T, DatabaseError>> + Send + 'a>>;

/// The free read side: NIP-01 over WebSocket, on streams the relay accepted.
#[derive(Debug, Clone)]
pub(crate) struct ReadSide {
    framework: LocalRelay,
    /// For the stored answers the gate gives itself (see `gate`).
    store: Store,
    edge: EdgeSlot,
    write_carriage: Option<Carriage>,
    connections: Arc<Semaphore>,
    /// What every connection's gate delivers live events from.
    live: gate::LiveFeed,
}

impl ReadSide {
    /// A read side that answers `REQ` from `store`, refuses `EVENT` towards
    /// the Write Edge in `edge` as it stands at the time, and holds at most
    /// `max_connections` connections at once.
    pub(crate) fn new(
        store: Store,
        edge: EdgeSlot,
        write_carriage: Option<Carriage>,
        max_connections: usize,
    ) -> Self {
        let framework = LocalRelay::builder()
            .database(StoredEvents(store.clone()))
            .write_policy(RefuseWrites)
            .build();
        Self {
            framework,
            store,
            edge,
            write_carriage,
            // More permits than a semaphore can hold is no cap at all.
            connections: Arc::new(Semaphore::new(max_connections.min(Semaphore::MAX_PERMITS))),
            live: gate::LiveFeed::new(),
        }
    }

    /// Speak NIP-01 with `peer` on `stream`, a connection already upgraded to
    /// WebSocket, until either side closes it. A connection past the cap is
    /// closed with 1013 instead.
    pub(crate) async fn serve<S>(&self, stream: S, peer: SocketAddr) -> Result<(), RelayError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let Ok(_held) = Arc::clone(&self.connections).try_acquire_owned() else {
            eprintln!("read: connection from {peer} refused: the connection cap is reached");
            return gate::refuse_full(stream).await;
        };
        let framework = self.framework.clone();
        let store = self.store.clone();
        let refusal = gate::Refusal {
            edge: self.edge.clone(),
            write_carriage: self.write_carriage,
        };
        gate::through(stream, refusal, store, &self.live, move |pipe| async move {
            framework
                .take_connection(pipe, peer)
                .await
                .map_err(|error| RelayError::ReadSide(error.to_string()))
        })
        .await
    }

    /// Deliver `event` to every open subscription it matches. Nothing is
    /// saved: the caller has saved it, or it is not to be kept.
    pub(crate) fn deliver(&self, event: &VerifiedEvent) {
        self.live.publish(event.event());
    }
}

/// The write policy: every `EVENT` that reaches the framework is refused. The
/// gate answers each one first, so this is the backstop, not the words a
/// client reads.
#[derive(Debug)]
struct RefuseWrites;

impl WritePolicy for RefuseWrites {
    fn admit_event<'a>(
        &'a self,
        _event: &'a Event,
        _peer: &'a SocketAddr,
    ) -> Pin<Box<dyn Future<Output = WritePolicyResult> + Send + 'a>> {
        Box::pin(async {
            WritePolicyResult::reject(
                MachineReadablePrefix::Restricted,
                "writes arrive on the write port",
            )
        })
    }
}

/// The store as the framework sees it: something to read. Its trait takes an
/// unverified [`Event`] to save, so every write is refused here; if the write
/// policy above were ever removed, a WebSocket `EVENT` would still not reach
/// the file.
#[derive(Debug)]
struct StoredEvents(Store);

impl StoredEvents {
    async fn matching(&self, filter: Filter) -> Result<Vec<Event>, DatabaseError> {
        self.0.query(filter).await.map_err(DatabaseError::storage)
    }
}

impl NostrDatabase for StoredEvents {
    fn backend(&self) -> &'static str {
        "sqlite"
    }

    fn features(&self) -> Features {
        Features {
            persistent: true,
            event_expiration: false,
            full_text_search: false,
            request_to_vanish: false,
        }
    }

    fn save_event<'a>(&'a self, _event: &'a Event) -> Answer<'a, SaveEventStatus> {
        Box::pin(async { Ok(SaveEventStatus::Rejected(RejectedReason::Other)) })
    }

    fn check_id<'a>(&'a self, event_id: &'a EventId) -> Answer<'a, DatabaseEventStatus> {
        Box::pin(async {
            Ok(match self.event_by_id(event_id).await? {
                Some(_) => DatabaseEventStatus::Saved,
                None => DatabaseEventStatus::NotExistent,
            })
        })
    }

    fn event_by_id<'a>(&'a self, event_id: &'a EventId) -> Answer<'a, Option<Event>> {
        Box::pin(async {
            let found = self.matching(Filter::new().id(*event_id).limit(1)).await?;
            Ok(found.into_iter().next())
        })
    }

    fn count(&self, filter: Filter) -> Answer<'_, usize> {
        Box::pin(async { Ok(self.matching(filter).await?.len()) })
    }

    fn query(&self, filter: Filter) -> Answer<'_, BTreeSet<Event>> {
        Box::pin(async { Ok(self.matching(filter).await?.into_iter().collect()) })
    }

    fn delete(&self, _filter: Filter) -> Answer<'_, ()> {
        Box::pin(async { Err(DatabaseError::unsupported(READ_ONLY)) })
    }

    fn wipe(&self) -> Answer<'_, ()> {
        Box::pin(async { Err(DatabaseError::unsupported(READ_ONLY)) })
    }
}

const READ_ONLY: &str = "the framework reads the relay's store and does not write it";
