//! The address this relay's writes are paid at, confirmed. It exists so that
//! "the relay refuses to advertise an address the connector does not
//! terminate" is a fact about types rather than about how the document
//! happens to be rendered (#185, invariant 4).
//!
//! A connector publishes its routes as prefix and price and deliberately not
//! their handlers (connector rule ND-08), so from its self-description alone
//! there is no telling which route reaches this relay's `POST /write`. The
//! relay is told that one thing, `TOON_WRITE_ILP_ADDRESS`, and being told is
//! not enough to advertise it: a relay pointed at a prefix its connector does
//! not terminate would send every client's money to a route that refuses it.
//!
//! What confirming establishes is that the connector publishes a route at
//! exactly this address. Its self-description lists the routes it forwards
//! beside the ones it terminates and does not tell them apart, so "this route
//! ends at this relay" cannot be read from it at all. For the bundle this repo
//! deploys, `deploy/bundle.test.ts` holds the configured address equal to the
//! prefix whose handler is this relay's `POST /write`.
//!
//! [`TerminatedRoute::confirm`] is the only constructor: the field is
//! private, the type is not `Deserialize`, and there is no conversion from a
//! string. `tests/compile_fail/` shows each of the other ways in failing to
//! build.

use connector_domain::node::{NodeSelfDescription, RoutePrice};

use crate::RelayError;

/// A route the connector publishes, at the address this relay was configured
/// with.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminatedRoute {
    /// The connector's own entry, so the address, the price and the carriage
    /// read from it are the confirmed route's and no other's.
    published: RoutePrice,
}

impl TerminatedRoute {
    /// Confirm that `configured`, the address this relay was told its writes
    /// are paid at, is the prefix of a route `connector` publishes.
    ///
    /// The match is exact. A longer or shorter prefix is another route, with
    /// another handler and another price, and an empty address is no address
    /// whatever the connector publishes. Otherwise the answer is
    /// [`RelayError::RouteNotTerminated`], which names the prefixes the
    /// connector does terminate.
    pub fn confirm(configured: &str, connector: &NodeSelfDescription) -> Result<Self, RelayError> {
        connector
            .routes
            .iter()
            .find(|route| !configured.is_empty() && route.prefix == configured)
            .map(|route| Self {
                published: route.clone(),
            })
            .ok_or_else(|| RelayError::RouteNotTerminated {
                address: configured.to_string(),
                terminated: connector
                    .routes
                    .iter()
                    .map(|route| route.prefix.clone())
                    .collect(),
            })
    }

    /// The address to advertise: the route's prefix, in the connector's
    /// spelling.
    pub fn address(&self) -> &str {
        &self.published.prefix
    }

    /// The route's price as the connector publishes it: a decimal string.
    pub(crate) fn price(&self) -> &str {
        &self.published.price
    }

    /// The route's price per KiB, when the connector prices it by size.
    pub(crate) fn price_per_kib(&self) -> Option<&str> {
        self.published.price_per_kib.as_deref()
    }

    /// The carriage the connector says this route requires, when it says.
    pub(crate) fn required_transport(&self) -> Option<&str> {
        self.published.required_transport.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// The body of a connector's `GET /ilp` that terminates `prefixes`.
    fn terminating(prefixes: &[&str]) -> NodeSelfDescription {
        let routes: Vec<_> = prefixes
            .iter()
            .map(|prefix| json!({ "prefix": prefix, "price": "1000" }))
            .collect();
        serde_json::from_value(json!({
            "peerCarriages": [],
            "routes": routes,
            "supportedVersions": [1],
            "defaultVersion": 1,
        }))
        .expect("the document is one a connector publishes")
    }

    #[test]
    fn an_address_the_connector_terminates_is_confirmed_in_the_connectors_spelling() {
        let connector = terminating(&["g.toon.relay.store", "g.toon.relay"]);
        let route = TerminatedRoute::confirm("g.toon.relay", &connector)
            .expect("the connector terminates the address");
        assert_eq!(route.address(), "g.toon.relay");
        assert_eq!(route.price(), "1000");
        assert_eq!(route.required_transport(), None);
    }

    #[test]
    fn an_address_the_connector_does_not_terminate_is_refused_naming_what_it_does() {
        let connector = terminating(&["g.toon.relay", "g.toon.relay.store"]);
        let error = TerminatedRoute::confirm("g.toon.elsewhere", &connector)
            .expect_err("the connector does not terminate the address");
        assert!(matches!(
            &error,
            RelayError::RouteNotTerminated { address, terminated }
                if address == "g.toon.elsewhere"
                    && terminated == &["g.toon.relay", "g.toon.relay.store"]
        ));
        assert_eq!(
            error.to_string(),
            "the connector does not terminate `g.toon.elsewhere` — it terminates \
             g.toon.relay, g.toon.relay.store. Point TOON_WRITE_ILP_ADDRESS at the prefix \
             whose route reaches this relay's POST /write"
        );
    }

    #[test]
    fn a_longer_a_shorter_or_a_differently_cased_address_is_another_route() {
        let connector = terminating(&["g.toon.relay"]);
        for other in [
            "g.toon.relay.store",
            "g.toon",
            "G.TOON.RELAY",
            "g.toon.relay ",
            "",
        ] {
            assert!(
                matches!(
                    TerminatedRoute::confirm(other, &connector),
                    Err(RelayError::RouteNotTerminated { .. })
                ),
                "{other:?}"
            );
        }
    }

    #[test]
    fn an_empty_address_is_never_confirmed_even_against_an_empty_prefix() {
        assert!(matches!(
            TerminatedRoute::confirm("", &terminating(&["", "g.toon.relay"])),
            Err(RelayError::RouteNotTerminated { .. })
        ));
    }

    #[test]
    fn a_connector_with_no_routes_terminates_nothing() {
        let error = TerminatedRoute::confirm("g.toon.relay", &terminating(&[]))
            .expect_err("there is no route to confirm against");
        assert_eq!(
            error.to_string(),
            "the connector does not terminate `g.toon.relay` — it terminates nothing. Point \
             TOON_WRITE_ILP_ADDRESS at the prefix whose route reaches this relay's POST /write"
        );
    }
}
