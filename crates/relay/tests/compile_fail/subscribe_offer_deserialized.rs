//! A subscribe offer cannot be read out of JSON someone sent: the only way to
//! one is reading a connector's self-description.

use relay::SubscribeOffer;

fn from_wire(json: &str) -> Option<SubscribeOffer> {
    serde_json::from_str(json).ok()
}

fn main() {}
