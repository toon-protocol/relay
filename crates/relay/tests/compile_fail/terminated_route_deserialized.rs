//! A terminated route cannot be parsed from a file or a setting.

use relay::TerminatedRoute;

fn parse(json: &str) -> Option<TerminatedRoute> {
    serde_json::from_str(json).ok()
}

fn main() {}
