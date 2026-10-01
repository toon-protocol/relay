//! A verified event cannot be made by wrapping an event: the only way to one
//! is `VerifiedEvent::verify`.

use nostr::event::Event;
use relay::VerifiedEvent;

fn wrap(event: Event) -> VerifiedEvent {
    VerifiedEvent(event)
}

fn main() {}
