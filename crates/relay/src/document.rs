//! The Relay Information Document (NIP-11), served on the read port to a
//! client that asks for it by name.
//!
//! What is here is what the read side owns: the identity, the NIPs it is
//! tested to support, and the limits the gate enforces, stated from the same
//! constants so the document cannot promise what the gate does not do. `AUTH`
//! is not required and so is not advertised. The `toon` object, the
//! operator's name and description, and the NIP-40 claim arrive with the
//! connector edge and the configuration (#199, #200); a document without a
//! `toon` object is the one a relay serves when it cannot confirm its Write
//! Edge (story 26).

use serde_json::{Value, json};

use crate::Relay;
use crate::gate::{MAX_FILTERS, MAX_SUBSCRIPTIONS};

/// The media type a client names to ask for the document.
pub(crate) const CONTENT_TYPE: &str = "application/nostr+json";

const SOFTWARE: &str = "https://github.com/toon-protocol/relay";

/// The NIPs this relay is tested to support.
const SUPPORTED_NIPS: [u16; 4] = [1, 9, 11, 16];

/// Whether an `Accept` header names the document. Every other plain request
/// keeps its `426`.
pub(crate) fn is_asked_for(accept: Option<&str>) -> bool {
    accept.is_some_and(|accept| {
        accept.split(',').any(|part| {
            part.split(';')
                .next()
                .is_some_and(|media| media.trim().eq_ignore_ascii_case(CONTENT_TYPE))
        })
    })
}

pub(crate) fn build(relay: &Relay) -> Value {
    let paid = relay.read_side.edge().filter(|edge| edge.price() > 0);
    let mut document = json!({
        "pubkey": relay.identity.to_hex(),
        "supported_nips": SUPPORTED_NIPS,
        "software": SOFTWARE,
        "version": env!("CARGO_PKG_VERSION"),
        "limitation": {
            "payment_required": paid.is_some(),
            "restricted_writes": true,
            "max_subscriptions": MAX_SUBSCRIPTIONS,
            "max_filters": MAX_FILTERS,
            "auth_required": false,
        },
    });
    if let Some(edge) = paid {
        document["fees"] = json!({ "publication": [{ "amount": edge.price(), "unit": "uusdc" }] });
    }
    document
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_a_request_naming_the_media_type_asks_for_the_document() {
        for accept in [
            "application/nostr+json",
            "Application/Nostr+JSON; q=0.9",
            "text/html, application/nostr+json",
        ] {
            assert!(is_asked_for(Some(accept)), "{accept}");
        }
        for accept in ["*/*", "application/json", "text/html", ""] {
            assert!(!is_asked_for(Some(accept)), "{accept}");
        }
        assert!(!is_asked_for(None));
    }
}
