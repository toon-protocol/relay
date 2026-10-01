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

use crate::gate::{MAX_FILTERS, MAX_SUBSCRIPTIONS};
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

/// What the document needs of the relay besides the edge.
#[derive(Debug, Clone)]
pub(crate) struct Settings {
    pub(crate) pubkey: String,
    pub(crate) name: Option<String>,
    pub(crate) description: Option<String>,
    pub(crate) contact: Option<String>,
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
    max_subscriptions: usize,
    max_filters: usize,
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
            name: settings.name.clone(),
            description: settings.description.clone(),
            pubkey: settings.pubkey.clone(),
            contact: settings.contact.clone(),
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
                carriage: carriage(edge, settings.write_carriage).map(Carriage::as_str),
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

/// The carriage a client is told: the connector's, or the operator's where
/// the connector states none.
fn carriage(edge: &WriteEdge, write_carriage: Option<Carriage>) -> Option<Carriage> {
    edge.carriage().or(write_carriage)
}

/// What a WebSocket `EVENT` is refused with, after the `restricted: ` prefix
/// the framework writes. It is rendered from the same edge as the document,
/// so a relay never refuses a write towards one address while advertising
/// another, and a client can recover from the refusal alone: it names the
/// address, the connector and the price, and points at the document for the
/// sealing key. The words after the prefix are the TypeScript relay's, which
/// clients already match on.
pub(crate) fn write_refusal(edge: Option<&WriteEdge>, write_carriage: Option<Carriage>) -> String {
    let document =
        format!("this relay's NIP-11 document (GET its URL with Accept: {CONTENT_TYPE})");
    let Some(edge) = edge else {
        return format!(
            "writes require ILP payment, and this relay does not publish where — ask its \
             operator, then see {document}"
        );
    };
    let carriage = carriage(edge, write_carriage).map_or(String::new(), |carriage| {
        format!(" over {}", carriage.as_str())
    });
    let (address, connector) = (edge.ilp_address(), edge.connector_url());
    // A free relay still refuses the WebSocket write: the lane is the
    // restriction, not the price, and "requires payment" would be a lie.
    let lead = if edge.price() > 0 {
        format!(
            "writes require ILP payment — send this event to {address} through \
             {connector}{carriage}, {} {FEE_UNIT} per write",
            edge.price()
        )
    } else {
        format!(
            "writes arrive as TOON packets and this one is free — send this event to \
             {address} through {connector}{carriage}"
        )
    };
    format!("{lead}; the sealing key is in {document}")
}
