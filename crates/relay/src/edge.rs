//! Where a write to this relay is paid for: the Write Edge. It exists so that
//! "the Write Edge is read from the connector, never held here" is a fact
//! about types rather than about where the launcher happens to look (#185,
//! invariant 3).
//!
//! The relay speaks no ILP and holds no price; the connector in front of it
//! enforces payment. A second copy of the enforcer's facts is a copy that
//! drifts, and the failure is a relay advertising a price nobody charges or a
//! key nobody holds. So the relay asks: the connector serves its own facts,
//! free, on `GET /ilp` (connector ADR 0050), and every field here is read
//! from that document, in the connector's own types.
//!
//! [`WriteEdge::read`] is the only constructor: the fields are private, the
//! type is not `Deserialize`, and nothing hands out a mutable field. It takes
//! one thing that is not the connector's, the address the relay was
//! configured with, and that reaches the edge only as a
//! [`TerminatedRoute`], confirmed against the same document.
//! `tests/compile_fail/` shows the other ways in failing to build.
//!
//! What the type cannot say is where the document came from. The connector's
//! `NodeSelfDescription` can be built by anyone who holds its fields, so the
//! promise here is that an edge has no other source, and fetching that
//! document from the connector and nowhere else is the reader's part (#199).
//!
//! What is deliberately not here is the operator's carriage setting
//! (`TOON_WRITE_CARRIAGE`). It fills the connector's silence in the rendered
//! document, but it is configuration, so it is not a field of this type:
//! [`WriteEdge::carriage`] is what the connector said and nothing else.

use connector_domain::node::NodeSelfDescription;
use connector_domain::x402::X402BatchSettlementTerms;

use crate::{RelayError, TerminatedRoute};

/// The largest price that survives a JSON number in every client: 2^53 - 1,
/// JavaScript's `Number.MAX_SAFE_INTEGER`. The document carries the price as
/// a number, so a price above it is refused rather than rounded.
const MAX_PRICE: u64 = (1 << 53) - 1;

/// The connector's spelling of "this route accepts either carriage".
const EITHER_CARRIAGE: &str = "both";

/// What a client needs to pay for a write: where, sealed to whom, over what,
/// for how much, settled how.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteEdge {
    route: TerminatedRoute,
    connector_url: String,
    seal_key: String,
    carriage: Option<Carriage>,
    price: u64,
    settlement: Vec<Settlement>,
}

/// The client transport a route is pinned to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Carriage {
    /// ILP over HTTP: `POST` to the connector's `httpEndpoint`.
    Http,
    /// BTP, over the connector's WebSocket endpoint.
    Btp,
}

/// One settlement the connector accepts: a network and an asset on it, in
/// the connector's own spelling.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settlement {
    network: String,
    asset: String,
}

impl WriteEdge {
    /// Read the Write Edge for `configured`, the address this relay was told
    /// its writes are paid at, out of `connector`'s self-description.
    ///
    /// Every failure is a [`RelayError`] that says what the connector did not
    /// publish; the caller's part is to say so and advertise nothing.
    pub fn read(configured: &str, connector: &NodeSelfDescription) -> Result<Self, RelayError> {
        let connector_url =
            published(&connector.http_endpoint).ok_or(RelayError::ConnectorPublishesNoUrl)?;
        let seal_key = connector
            .edge_identity
            .as_ref()
            .map(|identity| identity.public_key.as_str())
            .filter(|key| !key.is_empty())
            .ok_or(RelayError::ConnectorPublishesNoSealKey)?;
        let route = TerminatedRoute::confirm(configured, connector)?;
        let price = whole_price(route.price()).ok_or_else(|| RelayError::RoutePriceNotWhole {
            address: route.address().to_string(),
            price: route.price().to_string(),
        })?;
        let carriage = match route.required_transport() {
            // The route says nothing of its own, so the node speaks for it.
            None | Some(EITHER_CARRIAGE) => {
                Carriage::pinned(connector.required_transport.as_deref())
            }
            route => Carriage::pinned(route),
        };
        let settlement = connector
            .batch_settlements
            .iter()
            .map(Settlement::accepted)
            .collect();
        Ok(Self {
            route,
            connector_url: connector_url.to_string(),
            seal_key: seal_key.to_string(),
            carriage,
            price,
            settlement,
        })
    }

    /// The route the connector terminates at this relay.
    pub fn route(&self) -> &TerminatedRoute {
        &self.route
    }

    /// The ILP address a write is paid at.
    pub fn ilp_address(&self) -> &str {
        self.route.address()
    }

    /// The connector's client edge: its `httpEndpoint`.
    pub fn connector_url(&self) -> &str {
        &self.connector_url
    }

    /// The key a write's payload is sealed to: the connector's
    /// `edgeIdentity.publicKey`.
    pub fn seal_key(&self) -> &str {
        &self.seal_key
    }

    /// The carriage the connector pins: the route's, and the node's where
    /// the route states none of its own. `None` when the connector pins
    /// neither, which includes its permissive `both`.
    pub fn carriage(&self) -> Option<Carriage> {
        self.carriage
    }

    /// The route's price for one write, in the asset's base units.
    ///
    /// It is the connector's `price`, which is the whole price of a flat
    /// route and the base of a sloped one. The slope (`pricePerKib`) is not
    /// carried, as the TypeScript relay does not carry it: the document has
    /// one number for a price.
    pub fn price(&self) -> u64 {
        self.price
    }

    /// What the connector accepts settlement in, in its order.
    pub fn settlement(&self) -> &[Settlement] {
        &self.settlement
    }
}

impl Carriage {
    /// The carriage a `requiredTransport` pins. The connector's permissive
    /// `both` is not a pin and neither is a spelling this relay does not
    /// know: both reach the document as silence, never as a carriage a
    /// client should honour. A route that requires something unknown is not
    /// answered for by the node either.
    fn pinned(required_transport: Option<&str>) -> Option<Self> {
        match required_transport {
            Some("http") => Some(Self::Http),
            Some("btp") => Some(Self::Btp),
            _ => None,
        }
    }

    /// The connector's spelling, which is the document's.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Btp => "btp",
        }
    }
}

impl Settlement {
    /// The network and asset of one of the connector's settlement terms. The
    /// rest of the terms are for a client that opens a channel, which reads
    /// them from the connector.
    fn accepted(terms: &X402BatchSettlementTerms) -> Self {
        let (network, asset) = match terms {
            X402BatchSettlementTerms::Evm(evm) => (&evm.network, &evm.asset),
            X402BatchSettlementTerms::Solana(solana) => (&solana.network, &solana.asset),
        };
        Self {
            network: network.clone(),
            asset: asset.clone(),
        }
    }

    /// The CAIP-2 network, as the connector spells it.
    pub fn network(&self) -> &str {
        &self.network
    }

    /// The asset on that network, as the connector spells it.
    pub fn asset(&self) -> &str {
        &self.asset
    }
}

/// A fact the connector published: present and not empty.
fn published(fact: &Option<String>) -> Option<&str> {
    fact.as_deref().filter(|value| !value.is_empty())
}

/// A route's price as a whole number of base units: ASCII digits only, and
/// no more than a JSON number carries exactly.
fn whole_price(price: &str) -> Option<u64> {
    if price.is_empty() || !price.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    price.parse().ok().filter(|price| *price <= MAX_PRICE)
}

#[cfg(test)]
mod tests {
    use serde_json::{Value, json};

    use super::*;

    const SEAL_KEY: &str = "0x04abababababababababababababababababababababababababababababababab";

    /// The body of a `GET /ilp`, as the pinned connector publishes it for a
    /// node like the devnet relay's, with `overrides` replacing whole
    /// top-level keys (`null` removes one).
    fn connector(overrides: Value) -> NodeSelfDescription {
        let mut document = json!({
            "ilpAddresses": ["g.toon.relay"],
            "httpEndpoint": "https://relay.example/ilp",
            "btpEndpoint": "wss://relay.example/btp",
            "peerCarriages": ["btp"],
            "edgeIdentity": { "keyId": "edge-1", "publicKey": SEAL_KEY },
            "batchSettlements": [
                {
                    "network": "eip155:84532",
                    "asset": "0x036cbd53842c5426634e7929541ec2318f3dcf7e",
                    "payTo": "0x1111111111111111111111111111111111111111",
                    "receiverAuthorizer": "0x1111111111111111111111111111111111111111",
                    "withdrawDelay": 86400,
                    "name": "USDC",
                    "version": "2",
                    "assetTransferMethod": "eip3009",
                    "facilitator": "https://facilitator.example"
                },
                {
                    "network": "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
                    "asset": "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU",
                    "payTo": "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin",
                    "feePayer": "9xQeWvG816bUx9EPjHmaT23yvVM2ZWbrrpZb9PusVFin",
                    "withdrawDelay": 86400,
                    "tokenProgram": "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA",
                    "minDeposit": "1000",
                    "sponsorEndpoint": "https://relay.example/sponsor"
                }
            ],
            "routes": [
                { "prefix": "g.toon.relay", "price": "1000", "requiredTransport": "btp" },
                { "prefix": "g.toon.relay.ephemeral", "price": "0" },
                { "prefix": "g.toon.relay.store", "price": "2000", "pricePerKib": "10" }
            ],
            "supportedVersions": [1],
            "defaultVersion": 1
        });
        let fields = document.as_object_mut().expect("the document is an object");
        for (key, value) in overrides.as_object().expect("overrides are an object") {
            if value.is_null() {
                fields.remove(key);
            } else {
                fields.insert(key.clone(), value.clone());
            }
        }
        serde_json::from_value(document).expect("the document is one a connector publishes")
    }

    /// A connector whose only route is the relay's, with `route` merged in.
    fn connector_with_route(route: Value, node: Value) -> NodeSelfDescription {
        let mut entry = json!({ "prefix": "g.toon.relay", "price": "1000" });
        let fields = entry.as_object_mut().expect("the route is an object");
        for (key, value) in route.as_object().expect("the route's fields are an object") {
            fields.insert(key.clone(), value.clone());
        }
        let mut overrides = json!({ "routes": [entry] });
        let fields = overrides.as_object_mut().expect("overrides are an object");
        for (key, value) in node.as_object().expect("the node's fields are an object") {
            fields.insert(key.clone(), value.clone());
        }
        connector(overrides)
    }

    #[test]
    fn every_field_of_the_edge_is_the_connectors_own() {
        let edge = WriteEdge::read("g.toon.relay", &connector(json!({})))
            .expect("the connector terminates the address and publishes its edge");

        assert_eq!(edge.ilp_address(), "g.toon.relay");
        assert_eq!(edge.route().address(), "g.toon.relay");
        assert_eq!(edge.connector_url(), "https://relay.example/ilp");
        assert_eq!(edge.seal_key(), SEAL_KEY);
        assert_eq!(edge.carriage(), Some(Carriage::Btp));
        assert_eq!(edge.price(), 1000);
        let settlement: Vec<_> = edge
            .settlement()
            .iter()
            .map(|accepted| (accepted.network(), accepted.asset()))
            .collect();
        assert_eq!(
            settlement,
            [
                ("eip155:84532", "0x036cbd53842c5426634e7929541ec2318f3dcf7e"),
                (
                    "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1",
                    "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU"
                ),
            ]
        );
    }

    #[test]
    fn the_price_and_carriage_are_those_of_the_configured_route_and_no_other() {
        let edge = WriteEdge::read("g.toon.relay.ephemeral", &connector(json!({})))
            .expect("the connector terminates the free lane too");
        assert_eq!(edge.ilp_address(), "g.toon.relay.ephemeral");
        assert_eq!(edge.price(), 0);
        assert_eq!(edge.carriage(), None);
    }

    #[test]
    fn the_routes_carriage_wins_over_the_nodes() {
        let connector = connector_with_route(
            json!({ "requiredTransport": "http" }),
            json!({ "requiredTransport": "btp" }),
        );
        let edge = WriteEdge::read("g.toon.relay", &connector).expect("an edge");
        assert_eq!(edge.carriage(), Some(Carriage::Http));
        assert_eq!(Carriage::Http.as_str(), "http");
        assert_eq!(Carriage::Btp.as_str(), "btp");
    }

    #[test]
    fn a_route_that_pins_nothing_falls_to_the_node() {
        for route in [json!({}), json!({ "requiredTransport": "both" })] {
            let connector = connector_with_route(route, json!({ "requiredTransport": "btp" }));
            let edge = WriteEdge::read("g.toon.relay", &connector).expect("an edge");
            assert_eq!(edge.carriage(), Some(Carriage::Btp));
        }
    }

    #[test]
    fn a_route_that_requires_a_carriage_this_relay_does_not_know_states_none() {
        let connector = connector_with_route(
            json!({ "requiredTransport": "quic" }),
            json!({ "requiredTransport": "btp" }),
        );
        let edge = WriteEdge::read("g.toon.relay", &connector).expect("an edge");
        assert_eq!(edge.carriage(), None);
    }

    #[test]
    fn a_connector_that_pins_no_carriage_states_none() {
        for node in [
            json!({}),
            json!({ "requiredTransport": "both" }),
            json!({ "requiredTransport": "carrier-pigeon" }),
            json!({ "requiredTransport": "BTP" }),
        ] {
            let connector = connector_with_route(json!({}), node);
            let edge = WriteEdge::read("g.toon.relay", &connector).expect("an edge");
            assert_eq!(edge.carriage(), None);
        }
    }

    #[test]
    fn a_connector_that_accepts_no_settlement_has_an_edge_with_none() {
        let edge = WriteEdge::read(
            "g.toon.relay",
            &connector(json!({ "batchSettlements": null })),
        )
        .expect("settlement terms are not what makes an edge");
        assert!(edge.settlement().is_empty());
    }

    #[test]
    fn a_connector_with_no_url_has_no_edge() {
        for overrides in [
            json!({ "httpEndpoint": null }),
            json!({ "httpEndpoint": "" }),
        ] {
            let error = WriteEdge::read("g.toon.relay", &connector(overrides))
                .expect_err("there is no URL to send clients to");
            assert!(matches!(error, RelayError::ConnectorPublishesNoUrl));
        }
    }

    #[test]
    fn a_connector_with_no_sealing_key_has_no_edge() {
        for overrides in [
            json!({ "edgeIdentity": null }),
            json!({ "edgeIdentity": { "keyId": "edge-1", "publicKey": "" } }),
        ] {
            let error = WriteEdge::read("g.toon.relay", &connector(overrides))
                .expect_err("there is no key to seal a write to");
            assert!(matches!(error, RelayError::ConnectorPublishesNoSealKey));
        }
    }

    #[test]
    fn an_address_the_connector_does_not_terminate_has_no_edge() {
        let error = WriteEdge::read("g.toon.elsewhere", &connector(json!({})))
            .expect_err("the connector does not terminate the address");
        assert!(matches!(
            error,
            RelayError::RouteNotTerminated { address, .. } if address == "g.toon.elsewhere"
        ));
    }

    #[test]
    fn a_price_that_is_not_a_whole_number_a_json_number_can_carry_has_no_edge() {
        for price in [
            "",
            "10.5",
            "-1",
            "+1",
            "1e3",
            "0x10",
            " 1000",
            "9007199254740992",
        ] {
            let connector = connector_with_route(json!({ "price": price }), json!({}));
            let error = WriteEdge::read("g.toon.relay", &connector)
                .expect_err("the price is not a whole number of base units");
            assert!(
                matches!(
                    &error,
                    RelayError::RoutePriceNotWhole { address, price: refused }
                        if address == "g.toon.relay" && refused == price
                ),
                "{price:?}"
            );
        }
    }

    #[test]
    fn the_largest_price_a_json_number_can_carry_is_read_exactly() {
        let connector = connector_with_route(json!({ "price": "9007199254740991" }), json!({}));
        let edge = WriteEdge::read("g.toon.relay", &connector).expect("an edge");
        assert_eq!(edge.price(), 9_007_199_254_740_991);
    }

    #[test]
    fn a_refusal_says_what_the_connector_did_not_publish() {
        assert_eq!(
            RelayError::ConnectorPublishesNoUrl.to_string(),
            "the connector publishes no `httpEndpoint`, so it has no URL to send clients to \
             (its [node] section is unset)"
        );
        assert_eq!(
            RelayError::ConnectorPublishesNoSealKey.to_string(),
            "the connector publishes no `edgeIdentity.publicKey`, so there is no key for a \
             client to seal a write to"
        );
        assert_eq!(
            RelayError::RoutePriceNotWhole {
                address: "g.toon.relay".to_string(),
                price: "10.5".to_string(),
            }
            .to_string(),
            "the connector prices `g.toon.relay` at \"10.5\", which is not a whole number of uusdc"
        );
    }

    #[test]
    fn a_refusal_names_the_edge_a_paying_client_needs() {
        let edge = WriteEdge::read("g.toon.relay", &connector(json!({}))).expect("an edge");
        let refusal = crate::gate::write_refusal(Some(&edge));
        assert_eq!(
            refusal,
            "restricted: writes require ILP payment — send this event to g.toon.relay \
             through https://relay.example/ilp over btp, 1000 uusdc per write; \
             the sealing key is in this relay's NIP-11 document (GET its URL with \
             Accept: application/nostr+json)"
        );
    }

    #[test]
    fn a_refusal_on_a_free_route_does_not_claim_payment_is_required() {
        let connector = connector_with_route(json!({ "price": "0" }), json!({}));
        let edge = WriteEdge::read("g.toon.relay", &connector).expect("a free edge");
        let refusal = crate::gate::write_refusal(Some(&edge));
        assert!(
            refusal.starts_with("restricted: writes arrive as TOON packets and this one is free — send this event to g.toon.relay"),
            "{refusal}"
        );
        assert!(!refusal.contains("uusdc"), "{refusal}");
    }

    #[test]
    fn a_refusal_without_an_edge_says_the_relay_does_not_publish_where() {
        assert_eq!(
            crate::gate::write_refusal(None),
            "restricted: writes require ILP payment, and this relay does not publish where \
             — ask its operator, then see this relay's NIP-11 document (GET its URL with \
             Accept: application/nostr+json)"
        );
    }
}
