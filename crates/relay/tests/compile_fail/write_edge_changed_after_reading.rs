//! A Write Edge cannot be changed once read: no field of it can be replaced
//! with a value that did not come from the connector.

use relay::{Carriage, WriteEdge};

fn pin(mut edge: WriteEdge) -> WriteEdge {
    edge.carriage = Some(Carriage::Btp);
    edge.price = 0;
    edge
}

fn main() {}
