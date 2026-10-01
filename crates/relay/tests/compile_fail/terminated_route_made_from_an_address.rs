//! The address the relay was configured with is not a terminated route, and
//! does not convert into one: it has to be confirmed.

use relay::TerminatedRoute;

fn advertise(configured: String) -> TerminatedRoute {
    configured.into()
}

fn main() {}
