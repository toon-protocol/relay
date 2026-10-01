//! A payment statement cannot be assembled from a payer, an amount and a
//! chain: the only way to one is the paid-write handler reading the
//! connector's three headers.

use relay::{Chain, PaymentStatement};

fn claim(payer: String, amount: String) -> PaymentStatement {
    PaymentStatement {
        payer,
        amount,
        chain: Chain::Evm,
    }
}

fn main() {}
