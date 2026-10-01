//! A Write Edge cannot be assembled from values the relay holds, such as its
//! configuration: the only way to one is reading a connector's
//! self-description.

use relay::{TerminatedRoute, WriteEdge};

fn configured(route: TerminatedRoute, connector_url: String, seal_key: String) -> WriteEdge {
    WriteEdge {
        route,
        connector_url,
        seal_key,
        carriage: None,
        price: 1000,
        settlement: Vec::new(),
    }
}

fn main() {}
