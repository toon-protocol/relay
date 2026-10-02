//! A subscribe offer cannot be assembled from values the relay holds, such as
//! its configuration: its price is the route's, so the only way to one is
//! reading a connector's self-description.

use relay::{SubscribeOffer, TerminatedRoute};

fn configured(route: TerminatedRoute) -> SubscribeOffer {
    SubscribeOffer {
        route,
        price: 1000,
        carriage: None,
    }
}

fn main() {}
