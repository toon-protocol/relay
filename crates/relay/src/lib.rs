//! The TOON relay, rebuilt in Rust as a drop-in replacement for the
//! TypeScript image (#185).
//!
//! What is built so far is the path the design rests on (#193): an event
//! posted to `POST /write` is verified, saved to the SQLite file the
//! TypeScript relay already writes, and served to a client that sends `REQ`
//! over WebSocket, stored or live. Beside it, `GET /health`, and the types
//! that hold the relay's rules (#194): a verified event, a payment statement,
//! a Write Edge and a terminated route, each with one constructor. The whole
//! command line and environment of the TypeScript relay is accepted (#200),
//! `/metrics` is served, and the process stops cleanly on SIGINT and SIGTERM.
//! The edge is read from the connector's `GET /ilp` in the background (#199)
//! and rendered into the Relay Information Document on the read port, and
//! into the refusal a WebSocket `EVENT` gets. Every other surface in #185's
//! compatibility contract is a later slice, and until it lands the
//! conformance suite lists it as an expected failure for this
//! implementation.
//!
//! It is a library only so that the routers can be driven in a test without
//! the binary, and so the invariant types can be shown not to compile when
//! misused. Nothing is published and nothing outside this workspace imports
//! it.

mod auth;
mod clock;
mod config;
mod connector;
mod document;
mod edge;
mod error;
mod health;
mod metrics;
mod read;
mod read_side;
mod route;
mod session;
mod store;
mod verified;
mod version;
mod write;

pub use config::{Config, EdgeSettings, Invocation, USAGE};
pub use edge::{Carriage, Settlement, WriteEdge};
pub use error::RelayError;
pub use route::TerminatedRoute;
pub use store::{Query, Retention, Saved, Store};
pub use verified::VerifiedEvent;
/// The version the relay reports: its release handle, or the crate version
/// when built without one. See the `version` module.
pub fn version() -> &'static str {
    version::VERSION
}

pub use write::{Chain, PaymentStatement};

use std::sync::Arc;

use axum::Router;
use axum::routing::{any, get, post};
use nostr::key::PublicKey;

use crate::connector::{EdgeSlot, Intervals};
use crate::document::Settings;
use crate::metrics::Metrics;
use crate::read_side::ReadSide;

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
    metrics: Metrics,
    ephemeral: Arc<write::Lane>,
    log_writes: bool,
    reaper: Reaper,
}

/// Whether the reaper runs, when, and how long it lets an expired event stay.
#[derive(Debug, Clone, Copy)]
struct Reaper {
    enforce_expiration: bool,
    grace_seconds: u64,
    interval_seconds: u64,
}

impl Relay {
    /// Open the relay's database, creating the data directory and the file if
    /// they are missing. Nothing is bound: the caller serves the routers.
    pub fn open(config: &Config) -> Result<Self, RelayError> {
        std::fs::create_dir_all(&config.data_dir).map_err(|source| RelayError::DataDir {
            path: config.data_dir.clone(),
            source,
        })?;
        let store = Store::open_with(
            &config.database_path(),
            Retention {
                enforce_expiration: config.enforce_expiration,
                blocked_event_ids: config.blocked_event_ids.iter().cloned().collect(),
            },
        )?;
        let edge = EdgeSlot::default();
        let auth = config.auth_policy();
        Ok(Self {
            identity: config.identity,
            edge: edge.clone(),
            document: Settings {
                pubkey: config.identity.to_hex(),
                name: config.relay_name.clone(),
                description: config.relay_description.clone(),
                contact: config.relay_contact.clone(),
                write_carriage: config.write_carriage,
                enforce_expiration: config.enforce_expiration,
                nip42: auth.is_some(),
            },
            read_side: ReadSide::new(
                store.clone(),
                edge.clone(),
                config.write_carriage,
                usize::try_from(config.max_connections).unwrap_or(usize::MAX),
                auth,
            ),
            metrics: Metrics::new(config),
            ephemeral: Arc::new(write::Lane::new(config)),
            log_writes: config.log_writes,
            store,
            reaper: Reaper {
                enforce_expiration: config.enforce_expiration,
                grace_seconds: config.expiration_reap_grace_seconds,
                interval_seconds: config.expiration_reap_interval_seconds,
            },
        })
    }

    /// Start the NIP-40 reaper: one sweep now, then one every configured
    /// interval, each deleting what expired longer ago than the grace
    /// period. An interval of zero disables it, and so does turning
    /// expiration enforcement off: a relay still serving expired events must
    /// not be quietly deleting them. Either way nothing is started.
    ///
    /// Must be called inside a Tokio runtime. Dropping the handle detaches
    /// the task; aborting it stops the reaper.
    pub fn spawn_reaper(&self) -> Option<tokio::task::JoinHandle<()>> {
        let Reaper {
            enforce_expiration,
            grace_seconds,
            interval_seconds,
        } = self.reaper;
        if !enforce_expiration || interval_seconds == 0 {
            return None;
        }
        let store = self.store.clone();
        Some(tokio::spawn(async move {
            // The first tick is immediate: the boot sweep. A sweep that
            // overruns delays the next rather than bunching them.
            let mut sweeps =
                tokio::time::interval(std::time::Duration::from_secs(interval_seconds));
            sweeps.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                sweeps.tick().await;
                match store.reap_expired(grace_seconds).await {
                    Ok(0) => {}
                    Ok(removed) => println!("reaper removed {removed} expired event(s)"),
                    Err(error) => eprintln!("reaper failed: {error}"),
                }
            }
        }))
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
        let connector = config.edge.clone()?;
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
    ///
    /// Serve it with connection info, as the read router is, so the ephemeral
    /// lane's rate limit knows its callers apart.
    pub fn write_router(&self) -> Router {
        Router::new()
            .route("/health", get(health::health))
            .route("/metrics", get(metrics::metrics))
            .route("/write", post(write::write))
            .route("/write-ephemeral", post(write::write_ephemeral))
            .with_state(self.clone())
    }

    /// Everything served on the read port: the NIP-01 WebSocket, the Relay
    /// Information Document for a request that asks for it, and `426` for any
    /// other request that is not an upgrade.
    ///
    /// Serve it with upgrades enabled (`axum::serve` does) and with connection
    /// info, so the read side knows its peers.
    pub fn read_router(&self) -> Router {
        Router::new()
            .fallback(any(read::read))
            .with_state(self.clone())
    }
}
