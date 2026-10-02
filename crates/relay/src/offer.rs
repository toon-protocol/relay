//! What a subscribe packet costs and where it is paid: the paid live feed's
//! counterpart of the Write Edge (#215, the draft NIP's `toon_subscription`).
//!
//! The address and the broadcast price are the relay's own settings. The
//! subscribe price is not: it is the price of the route the connector
//! publishes at that address, read from the connector's own self-description
//! like every fact about where a payment goes, and never held here. So an
//! offer is only ever built from a [`NodeSelfDescription`], and a route the
//! connector does not terminate, does not price flat, or prices at nothing is
//! no offer at all: a relay must not advertise a route whose packets would
//! credit something other than its price.
//!
//! [`SubscribeOffer::read`] is the only constructor and the fields are private.

use connector_domain::node::NodeSelfDescription;

use crate::edge::{route_carriage, whole_price};
use crate::{Carriage, RelayError, TerminatedRoute};

/// The route a subscribe packet is paid at, as the connector publishes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubscribeOffer {
    route: TerminatedRoute,
    price: u64,
    carriage: Option<Carriage>,
}

impl SubscribeOffer {
    /// Read the offer for `configured`, the address this relay was told its
    /// subscribe route is paid at, out of `connector`'s self-description.
    pub fn read(configured: &str, connector: &NodeSelfDescription) -> Result<Self, RelayError> {
        let route = TerminatedRoute::confirm(configured, connector)?;
        let address = || route.address().to_string();
        if route.price_per_kib().is_some() {
            return Err(RelayError::SubscribeRouteNotFlat { address: address() });
        }
        let price = whole_price(route.price())
            .ok_or_else(|| RelayError::RoutePriceNotWhole {
                address: address(),
                price: route.price().to_string(),
            })
            .and_then(|price| {
                if price == 0 {
                    Err(RelayError::SubscribeRouteFree { address: address() })
                } else {
                    Ok(price)
                }
            })?;
        let carriage = route_carriage(&route, connector);
        Ok(Self {
            route,
            price,
            carriage,
        })
    }

    /// The address a subscribe packet is sent to.
    pub fn ilp_address(&self) -> &str {
        self.route.address()
    }

    /// What one packet credits, in base units: the route's flat price.
    pub fn price(&self) -> u64 {
        self.price
    }

    /// The carriage the route pins, when it pins one.
    pub fn carriage(&self) -> Option<Carriage> {
        self.carriage
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn connector(route: serde_json::Value) -> NodeSelfDescription {
        serde_json::from_value(json!({
            "peerCarriages": [],
            "routes": [{ "prefix": "g.toon.relay", "price": "1" }, route],
            "supportedVersions": [1],
            "defaultVersion": 1,
            "requiredTransport": "btp",
        }))
        .expect("a document a connector publishes")
    }

    #[test]
    fn the_price_and_carriage_are_those_of_the_route_the_connector_publishes() {
        let described = connector(json!({
            "prefix": "g.toon.relay.subscribe",
            "price": "1000",
            "requiredTransport": "http",
        }));
        let offer = SubscribeOffer::read("g.toon.relay.subscribe", &described).expect("an offer");
        assert_eq!(offer.ilp_address(), "g.toon.relay.subscribe");
        assert_eq!(offer.price(), 1000);
        assert_eq!(offer.carriage(), Some(Carriage::Http));

        // Silent route: the node's pin.
        let silent = connector(json!({ "prefix": "g.toon.relay.subscribe", "price": "5" }));
        let offer = SubscribeOffer::read("g.toon.relay.subscribe", &silent).expect("an offer");
        assert_eq!(offer.carriage(), Some(Carriage::Btp));
    }

    #[test]
    fn a_route_the_connector_does_not_terminate_is_no_offer() {
        let described = connector(json!({ "prefix": "g.toon.relay.other", "price": "1000" }));
        assert!(matches!(
            SubscribeOffer::read("g.toon.relay.subscribe", &described),
            Err(RelayError::RouteNotTerminated { .. })
        ));
    }

    #[test]
    fn a_route_that_is_not_a_flat_price_above_zero_is_no_offer() {
        let sloped = connector(json!({
            "prefix": "g.toon.relay.subscribe", "price": "1000", "pricePerKib": "10",
        }));
        assert!(matches!(
            SubscribeOffer::read("g.toon.relay.subscribe", &sloped),
            Err(RelayError::SubscribeRouteNotFlat { .. })
        ));
        let free = connector(json!({ "prefix": "g.toon.relay.subscribe", "price": "0" }));
        assert!(matches!(
            SubscribeOffer::read("g.toon.relay.subscribe", &free),
            Err(RelayError::SubscribeRouteFree { .. })
        ));
        let odd = connector(json!({ "prefix": "g.toon.relay.subscribe", "price": "1.5" }));
        assert!(matches!(
            SubscribeOffer::read("g.toon.relay.subscribe", &odd),
            Err(RelayError::RoutePriceNotWhole { .. })
        ));
    }
}
