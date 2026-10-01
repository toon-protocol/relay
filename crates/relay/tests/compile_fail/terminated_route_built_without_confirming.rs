//! A terminated route cannot be made by wrapping a route entry: the only way
//! to one is `TerminatedRoute::confirm`, against a connector's
//! self-description.

use connector_domain::node::RoutePrice;
use relay::TerminatedRoute;

fn wrap(published: RoutePrice) -> TerminatedRoute {
    TerminatedRoute { published }
}

fn main() {}
