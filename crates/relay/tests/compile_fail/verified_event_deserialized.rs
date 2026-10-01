//! A verified event cannot be read off the wire: parsing JSON proves nothing
//! about a signature.

use relay::VerifiedEvent;

fn parse(json: &str) -> Option<VerifiedEvent> {
    serde_json::from_str(json).ok()
}

fn main() {}
