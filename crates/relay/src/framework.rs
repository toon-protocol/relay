//! The adapter over the framework: `nostr-sdk`'s `local_relay`, which parses
//! NIP-01 messages, keeps subscriptions and fans events out to them.
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
//! - `EVENT` over WebSocket is refused by its write policy: writes are paid,
//!   and arrive on the write port. The refusal names the Write Edge once the
//!   relay knows it.
//! - It speaks to a connection through the gate (`gate.rs`), which holds the
//!   limits the framework would answer differently and the connection cap.
//! - `AUTH` is neither required nor advertised: NIP-42 is left off.

use std::collections::BTreeSet;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, PoisonError, RwLock};

use nostr::event::{Event, EventId};
use nostr::filter::Filter;
use nostr::message::MachineReadablePrefix;
use nostr_database::error::Error as DatabaseError;
use nostr_database::{
    DatabaseEventStatus, Features, NostrDatabase, RejectedReason, SaveEventStatus,
};
use nostr_sdk::local_relay::{LocalRelay, RateLimit, WritePolicy, WritePolicyResult};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::Semaphore;

use crate::{RelayError, Store, VerifiedEvent, WriteEdge, gate};

/// Where a client that sends `EVENT` over WebSocket is pointed for the rest.
const NIP11_HINT: &str = "this relay's NIP-11 document (GET its URL with \
     Accept: application/nostr+json)";

/// What a client that sends `EVENT` over WebSocket is told, after the
/// `restricted: ` prefix the framework writes: the TypeScript relay's words,
/// for a relay that does and does not know its Write Edge.
///
/// The sealing key is left to the document: it is too long for an `OK`
/// message a client may be logging a line at a time. A free route still
/// refuses the WebSocket write (the lane is the restriction, not the price),
/// so it must not claim payment is required.
pub(crate) fn write_refusal(edge: Option<&WriteEdge>) -> String {
    let Some(edge) = edge else {
        return format!(
            "writes require ILP payment, and this relay does not publish where \
             — ask its operator, then see {NIP11_HINT}"
        );
    };
    let carriage = edge.carriage().map_or(String::new(), |carriage| {
        format!(" over {}", carriage.as_str())
    });
    let address = edge.ilp_address();
    let url = edge.connector_url();
    let lead = match edge.price() {
        0 => format!(
            "writes arrive as TOON packets and this one is free \
             — send this event to {address} through {url}{carriage}"
        ),
        price => format!(
            "writes require ILP payment — send this event to {address} \
             through {url}{carriage}, {price} uusdc per write"
        ),
    };
    format!("{lead}; the sealing key is in {NIP11_HINT}")
}

/// What the relay knows of its Write Edge, shared with the write policy and
/// the document. Empty until the connector has been read.
type KnownEdge = Arc<RwLock<Option<WriteEdge>>>;

type Answer<'a, T> = Pin<Box<dyn Future<Output = Result<T, DatabaseError>> + Send + 'a>>;

/// The free read side: NIP-01 over WebSocket, on streams the relay accepted.
#[derive(Debug, Clone)]
pub(crate) struct ReadSide {
    framework: LocalRelay,
    edge: KnownEdge,
    connections: Arc<Semaphore>,
}

impl ReadSide {
    /// A read side that answers `REQ` from `store` and holds at most
    /// `max_connections` connections at once.
    pub(crate) fn new(store: Store, max_connections: usize) -> Self {
        let edge = KnownEdge::default();
        let framework = LocalRelay::builder()
            .database(StoredEvents(store))
            .write_policy(RefuseWrites(Arc::clone(&edge)))
            // The framework counts `EVENT`s before it asks the write policy,
            // and past its allowance answers `rate-limited` instead. Every
            // `EVENT` is refused anyway, so the count is lifted and the
            // refusal is always the one that says why.
            .rate_limit(RateLimit {
                notes_per_minute: u32::MAX,
                ..RateLimit::default()
            })
            .build();
        Self {
            framework,
            edge,
            // More permits than a semaphore can hold is no cap at all.
            connections: Arc::new(Semaphore::new(max_connections.min(Semaphore::MAX_PERMITS))),
        }
    }

    /// Record the Write Edge the connector states, or `None` when it states
    /// none: the refusal and the document name it from here on.
    pub(crate) fn know_edge(&self, edge: Option<WriteEdge>) {
        *self.edge.write().unwrap_or_else(PoisonError::into_inner) = edge;
    }

    /// The Write Edge the relay knows, if it knows one.
    pub(crate) fn edge(&self) -> Option<WriteEdge> {
        self.edge
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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
        gate::through(stream, move |pipe| async move {
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
        // `false` only says nobody is connected.
        self.framework.notify_event(event.event().clone());
    }
}

/// The write policy: every `EVENT` a client sends is refused, with the words
/// that name the Write Edge if the relay knows it.
#[derive(Debug)]
struct RefuseWrites(KnownEdge);

impl WritePolicy for RefuseWrites {
    fn admit_event<'a>(
        &'a self,
        _event: &'a Event,
        _peer: &'a SocketAddr,
    ) -> Pin<Box<dyn Future<Output = WritePolicyResult> + Send + 'a>> {
        Box::pin(async {
            let message = write_refusal(
                self.0
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .as_ref(),
            );
            WritePolicyResult::reject(MachineReadablePrefix::Restricted, message)
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
