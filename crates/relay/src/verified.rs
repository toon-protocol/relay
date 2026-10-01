//! An event whose id and signature were checked. It exists so that "nothing is
//! stored unless its signature was verified" is a fact about types rather than
//! about how the handlers happen to be written (#185, invariant 1).
//!
//! [`VerifiedEvent::verify`] is the only constructor: the field is private,
//! the type is not `Deserialize`, and nothing hands out a mutable event.
//! `tests/compile_fail/` shows each of the other ways in failing to build.

use nostr::event::Event;

use crate::RelayError;

/// An [`Event`] whose id is the hash of its content and whose signature is
/// its author's over that id.
#[derive(Debug, Clone)]
pub struct VerifiedEvent(Event);

impl VerifiedEvent {
    /// Check `event`'s id, then its signature.
    ///
    /// The two failures are told apart because the id is checked first: an
    /// event altered after signing is [`RelayError::EventIdMismatch`], and
    /// one whose id is honest but whose signature is not its author's is
    /// [`RelayError::EventSignatureInvalid`].
    pub fn verify(event: Event) -> Result<Self, RelayError> {
        if !event.verify_id() {
            return Err(RelayError::EventIdMismatch);
        }
        if !event.verify_signature() {
            return Err(RelayError::EventSignatureInvalid);
        }
        Ok(Self(event))
    }

    /// The event that was verified.
    pub fn event(&self) -> &Event {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Kind, Signature};
    use nostr::key::Keys;

    use super::*;

    fn signed(content: &str) -> Event {
        EventBuilder::new(Kind::TextNote, content)
            .finalize(&Keys::generate())
            .expect("a generated key signs a text note")
    }

    #[test]
    fn a_correctly_signed_event_is_verified_and_handed_back_unchanged() {
        let event = signed("hello");
        let verified = VerifiedEvent::verify(event.clone()).expect("the event is signed");
        assert_eq!(verified.event(), &event);
    }

    #[test]
    fn an_event_altered_after_signing_is_an_id_mismatch() {
        let mut event = signed("original");
        event.content = "tampered after signing".to_string();
        assert!(matches!(
            VerifiedEvent::verify(event),
            Err(RelayError::EventIdMismatch)
        ));
    }

    #[test]
    fn an_event_signed_by_someone_else_has_an_invalid_signature() {
        let mut event = signed("mine");
        event.sig = signed("someone else's").sig;
        assert!(matches!(
            VerifiedEvent::verify(event),
            Err(RelayError::EventSignatureInvalid)
        ));
    }

    #[test]
    fn a_signature_of_zeroes_is_invalid() {
        let mut event = signed("mine");
        event.sig = Signature::from_byte_array([0; 64]);
        assert!(matches!(
            VerifiedEvent::verify(event),
            Err(RelayError::EventSignatureInvalid)
        ));
    }
}
