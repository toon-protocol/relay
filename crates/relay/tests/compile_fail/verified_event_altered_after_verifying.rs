//! A verified event cannot be changed: nothing hands out the event inside it
//! mutably.

use relay::VerifiedEvent;

fn tamper(verified: VerifiedEvent) {
    verified.event().content = "altered after verifying".to_string();
}

fn main() {}
