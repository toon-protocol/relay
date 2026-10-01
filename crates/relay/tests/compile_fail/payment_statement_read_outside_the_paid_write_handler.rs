//! Only the paid-write handler reads a payment statement off a request: no
//! other handler, and nothing outside the relay, can claim a payment.

use axum::http::HeaderMap;
use relay::PaymentStatement;

fn claim(headers: &HeaderMap) -> Option<PaymentStatement> {
    PaymentStatement::stated_on(headers)
}

fn main() {}
