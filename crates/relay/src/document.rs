//! The Relay Information Document (NIP-11), served on the read port to a
//! request that asks for it by name (#199).
//!
//! It is rendered from the live Write Edge and from nothing else the relay
//! holds about payment: the `toon` object is the connector's own facts, and
//! `limitation.payment_required` and `fees` follow its price. When the edge is
//! unknown the document is served without a `toon` object, which is the truth.
//!
//! The one thing that is not the connector's is the operator's carriage
//! (`TOON_WRITE_CARRIAGE`). It fills the connector's silence here, at the
//! rendering, and never overrides what the connector states.

use serde::Serialize;

use crate::config::Description;
use crate::{Carriage, WriteEdge};

/// The media type NIP-11 gives the document.
pub(crate) const CONTENT_TYPE: &str = "application/nostr+json";

/// What `software` names: where the running code came from.
const SOFTWARE: &str = "https://github.com/toon-protocol/relay";

/// The NIPs implemented whatever the settings: 1 (the protocol), 9
/// (deletion), 11 (this document) and 16 (the ephemeral range).
const BASE_NIPS: [u16; 4] = [1, 9, 11, 16];

/// NIP-40, claimed while expiration is enforced.
const EXPIRATION_NIP: u16 = 40;

/// The unit a price is in: the connector's base units of its asset.
const FEE_UNIT: &str = "uusdc";

/// The read side's caps on one connection, as the TypeScript relay states
/// them.
const MAX_SUBSCRIPTIONS: u32 = 20;
const MAX_FILTERS: u32 = 10;

/// What the document needs of the relay besides the edge.
#[derive(Debug, Clone)]
pub(crate) struct Settings {
    pub(crate) pubkey: String,
    pub(crate) description: Description,
    pub(crate) write_carriage: Option<Carriage>,
    pub(crate) enforce_expiration: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct Document {
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    pubkey: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    contact: Option<String>,
    supported_nips: Vec<u16>,
    software: &'static str,
    version: &'static str,
    limitation: Limitation,
    #[serde(skip_serializing_if = "Option::is_none")]
    fees: Option<Fees>,
    #[serde(skip_serializing_if = "Option::is_none")]
    toon: Option<Toon>,
}

#[derive(Debug, Serialize)]
struct Limitation {
    payment_required: bool,
    restricted_writes: bool,
    max_subscriptions: u32,
    max_filters: u32,
    auth_required: bool,
}

#[derive(Debug, Serialize)]
struct Fees {
    publication: [Fee; 1],
}

#[derive(Debug, Serialize)]
struct Fee {
    amount: u64,
    unit: &'static str,
}

#[derive(Debug, Serialize)]
struct Toon {
    ilp_address: String,
    connector_url: String,
    connector_seal_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    carriage: Option<&'static str>,
    price: u64,
    settlement: Vec<Settled>,
}

#[derive(Debug, Serialize)]
struct Settled {
    network: String,
    asset: String,
}

impl Document {
    /// The document for `edge`, or for a relay that publishes none.
    pub(crate) fn render(settings: &Settings, edge: Option<&WriteEdge>) -> Self {
        let paid = edge.is_some_and(|edge| edge.price() > 0);
        let mut supported_nips = BASE_NIPS.to_vec();
        if settings.enforce_expiration {
            supported_nips.push(EXPIRATION_NIP);
        }
        Self {
            name: settings.description.name.clone(),
            description: settings.description.description.clone(),
            pubkey: settings.pubkey.clone(),
            contact: settings.description.contact.clone(),
            supported_nips,
            software: SOFTWARE,
            version: env!("CARGO_PKG_VERSION"),
            limitation: Limitation {
                payment_required: paid,
                restricted_writes: true,
                max_subscriptions: MAX_SUBSCRIPTIONS,
                max_filters: MAX_FILTERS,
                auth_required: false,
            },
            // Omitted, not 0, for a relay that charges nothing:
            // `payment_required` already says so.
            fees: edge.filter(|_| paid).map(|edge| Fees {
                publication: [Fee {
                    amount: edge.price(),
                    unit: FEE_UNIT,
                }],
            }),
            toon: edge.map(|edge| Toon {
                ilp_address: edge.ilp_address().to_string(),
                connector_url: edge.connector_url().to_string(),
                connector_seal_key: edge.seal_key().to_string(),
                carriage: edge
                    .carriage()
                    .or(settings.write_carriage)
                    .map(Carriage::as_str),
                price: edge.price(),
                settlement: edge
                    .settlement()
                    .iter()
                    .map(|accepted| Settled {
                        network: accepted.network().to_string(),
                        asset: accepted.asset().to_string(),
                    })
                    .collect(),
            }),
        }
    }
}
