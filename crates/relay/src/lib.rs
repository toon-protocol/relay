//! The TOON relay, rebuilt in Rust as a drop-in replacement for the
//! TypeScript image (#185).
//!
//! What is built so far is the path the design rests on (#193): an event
//! posted to `POST /write` is verified, saved to the SQLite file the
//! TypeScript relay already writes, and served to a client that sends `REQ`
//! over WebSocket, stored or live. Beside it, `GET /health`, and the types
//! that hold the relay's rules (#194): a verified event, a payment statement,
//! a Write Edge and a terminated route, each with one constructor. The edge
//! is read from the connector's `GET /ilp` in the background (#199) and
//! rendered into the Relay Information Document on the read port. Every other surface in #185's compatibility contract is a later
//! slice, and until it lands the conformance suite lists it as an expected
//! failure for this implementation.
//!
//! It is a library only so that the routers can be driven in a test without
//! the binary, and so the invariant types can be shown not to compile when
//! misused. Nothing is published and nothing outside this workspace imports
//! it.

mod clock;
mod config;
mod connector;
mod document;
mod edge;
mod error;
mod framework;
mod health;
mod read;
mod route;
mod store;
mod verified;
mod write;

pub use config::{Config, ConnectorConfig, Description};
pub use edge::{Carriage, Settlement, WriteEdge};
pub use error::RelayError;
pub use route::TerminatedRoute;
pub use store::{Saved, Store};
pub use verified::VerifiedEvent;
pub use write::{Chain, PaymentStatement};

use axum::Router;
use axum::routing::{any, get, post};
use nostr::key::PublicKey;

use crate::connector::{EdgeSlot, Intervals};
use crate::document::Settings;
use crate::framework::ReadSide;

/// A relay: its identity, its store, and the read side that serves the store
/// and receives what the write side accepts. Cheap to clone; every clone is
/// the same relay.
#[derive(Debug, Clone)]
pub struct Relay {
    identity: PublicKey,
    store: Store,
    read_side: ReadSide,
    edge: EdgeSlot,
    document: Settings,
}

impl Relay {
    /// Open the relay's database, creating the data directory and the file if
    /// they are missing. Nothing is bound: the caller serves the routers.
    pub fn open(config: &Config) -> Result<Self, RelayError> {
        std::fs::create_dir_all(&config.data_dir).map_err(|source| RelayError::DataDir {
            path: config.data_dir.clone(),
            source,
        })?;
        let store = Store::open(&config.database_path())?;
        Ok(Self {
            identity: config.identity,
            edge: EdgeSlot::default(),
            document: Settings {
                pubkey: config.identity.to_hex(),
                description: config.description.clone(),
                write_carriage: config.write_carriage,
                enforce_expiration: config.enforce_expiration,
            },
            read_side: ReadSide::new(store.clone()),
            store,
        })
    }

    /// Start reading the Write Edge from the connector, if one is configured,
    /// until the returned task is aborted. It returns at once: the connector
    /// is asked in the background, quickly while the edge is unknown and
    /// slowly once it is known, and no answer, or none, stops the relay.
    pub fn watch_connector(&self, config: &Config) -> Option<tokio::task::JoinHandle<()>> {
        self.watch_connector_at(config, Intervals::default())
    }

    fn watch_connector_at(
        &self,
        config: &Config,
        intervals: Intervals,
    ) -> Option<tokio::task::JoinHandle<()>> {
        let connector = config.connector.clone()?;
        Some(tokio::spawn(connector::watch(
            connector,
            intervals,
            self.edge.clone(),
        )))
    }

    /// Everything served on the write port.
    ///
    /// The caller binds: a router that does not own its port can be driven
    /// in a test with no listener.
    pub fn write_router(&self) -> Router {
        Router::new()
            .route("/health", get(health::health))
            .route("/write", post(write::write))
            .with_state(self.clone())
    }

    /// Everything served on the read port: the NIP-01 WebSocket, and `426`
    /// for a request that is not an upgrade.
    ///
    /// Serve it with upgrades enabled (`axum::serve` does) and with connection
    /// info, so the read side knows its peers.
    pub fn read_router(&self) -> Router {
        Router::new()
            .fallback(any(read::read))
            .with_state(self.clone())
    }
}
