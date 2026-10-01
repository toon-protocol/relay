//! A payment statement cannot be read out of a body: it is what the
//! connector states in its headers, never what a caller writes in JSON.

use relay::PaymentStatement;

fn parse(json: &str) -> Option<PaymentStatement> {
    serde_json::from_str(json).ok()
}

fn main() {}
