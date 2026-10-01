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
//!   and arrive on the write port.

use std::collections::BTreeSet;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;

use nostr::event::{Event, EventId};
use nostr::filter::Filter;
use nostr::message::MachineReadablePrefix;
use nostr_database::error::Error as DatabaseError;
use nostr_database::{
    DatabaseEventStatus, Features, NostrDatabase, RejectedReason, SaveEventStatus,
};
use nostr_sdk::local_relay::{LocalRelay, RateLimit, WritePolicy, WritePolicyResult};
use tokio::io::{AsyncRead, AsyncWrite};

use crate::connector::EdgeSlot;
use crate::document::write_refusal;
use crate::{Carriage, RelayError, Store, VerifiedEvent};

type Answer<'a, T> = Pin<Box<dyn Future<Output = Result<T, DatabaseError>> + Send + 'a>>;

/// The free read side: NIP-01 over WebSocket, on streams the relay accepted.
#[derive(Debug, Clone)]
pub(crate) struct ReadSide {
    framework: LocalRelay,
}

impl ReadSide {
    /// A read side that answers `REQ` from `store`, and refuses `EVENT`
    /// towards the Write Edge in `edge` as it stands at the time.
    pub(crate) fn new(store: Store, edge: EdgeSlot, write_carriage: Option<Carriage>) -> Self {
        let framework = LocalRelay::builder()
            .database(StoredEvents(store))
            .write_policy(RefuseWrites {
                edge,
                write_carriage,
            })
            // The framework counts `EVENT`s before it asks the write policy,
            // and past its allowance answers `rate-limited` instead. Every
            // `EVENT` is refused anyway, so the count is lifted and the
            // refusal is always the one that says why.
            .rate_limit(RateLimit {
                notes_per_minute: u32::MAX,
                ..RateLimit::default()
            })
            .build();
        Self { framework }
    }

    /// Speak NIP-01 with `peer` on `stream`, a connection already upgraded to
    /// WebSocket, until either side closes it.
    pub(crate) async fn serve<S>(&self, stream: S, peer: SocketAddr) -> Result<(), RelayError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        self.framework
            .take_connection(stream, peer)
            .await
            .map_err(|error| RelayError::ReadSide(error.to_string()))
    }

    /// Deliver `event` to every open subscription it matches. Nothing is
    /// saved: the caller has saved it, or it is not to be kept.
    pub(crate) fn deliver(&self, event: &VerifiedEvent) {
        // `false` only says nobody is connected.
        self.framework.notify_event(event.event().clone());
    }
}

/// The write policy: every `EVENT` a client sends is refused, with where
/// to send it instead.
#[derive(Debug)]
struct RefuseWrites {
    edge: EdgeSlot,
    write_carriage: Option<Carriage>,
}

impl WritePolicy for RefuseWrites {
    fn admit_event<'a>(
        &'a self,
        _event: &'a Event,
        _peer: &'a SocketAddr,
    ) -> Pin<Box<dyn Future<Output = WritePolicyResult> + Send + 'a>> {
        let refusal = write_refusal(self.edge.current().as_deref(), self.write_carriage);
        Box::pin(async { WritePolicyResult::reject(MachineReadablePrefix::Restricted, refusal) })
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
