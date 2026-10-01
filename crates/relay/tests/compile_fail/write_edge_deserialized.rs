//! A Write Edge cannot be parsed from a file or a setting: it is read from
//! the connector's self-description, in the connector's own types.

use relay::WriteEdge;

fn parse(json: &str) -> Option<WriteEdge> {
    serde_json::from_str(json).ok()
}

fn main() {}
