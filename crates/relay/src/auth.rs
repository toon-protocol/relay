//! NIP-42: a client proves which key it holds by signing a challenge the
//! relay sent its connection (#218).
//!
//! It is off unless the operator turns it on, so a relay that was never told
//! about it behaves as it always has: no challenge, nothing refused, and no 42
//! in its NIP-11 document. When on, every connection is sent a challenge as it
//! opens, a valid `AUTH` answering it authenticates the connection, and a
//! `REQ` that could return a kind the operator chose to restrict is closed
//! `auth-required:` until the connection has authenticated. Nothing else is
//! restricted: writes are paid and arrive on the write port, not here.
//!
//! An `AUTH` must name a relay, but which one is not checked: the relay is
//! not told the public URL it is reached at (it sits behind a proxy), so it
//! has nothing to compare the tag with. The challenge, unique to the
//! connection, is what ties an `AUTH` to this relay.
//!
//! A relay that sells its live feed (#215) challenges every connection whether
//! or not this is on, and is told its URL: there the `AUTH` is checked by
//! [`crate::proof`], which holds the `relay` tag to that URL, and the key it
//! proves counts here too.
//!
//! This module holds the rules and no connection state. The session keeps the
//! challenge and who has authenticated, and asks here whether an `AUTH` answers
//! it and whether a filter needs it.

use std::collections::{BTreeSet, HashSet};

use nostr::event::{Event, Kind};
use nostr::filter::Filter;
use nostr::key::{PublicKey, SecretKey};

use crate::VerifiedEvent;

/// How far an `AUTH` event's `created_at` may be from now, either way, in
/// seconds (NIP-42 suggests ten minutes).
const TOLERANCE_SECONDS: u64 = 600;

/// What a `CLOSED` for a `REQ` that needs authentication says.
pub(crate) const CLOSED_AUTH_REQUIRED: &str =
    "auth-required: this subscription needs authentication; answer the AUTH challenge first";

/// How a relay with NIP-42 on treats its connections.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct AuthPolicy {
    /// The kinds a connection must authenticate to read. Empty: none.
    required_kinds: BTreeSet<Kind>,
    /// A gift wrap (kind 1059) is readable only by a connection that has
    /// proven a key its `p` tags name.
    recipient_only: bool,
}

impl AuthPolicy {
    /// A policy that restricts reads of `kinds`.
    pub(crate) fn requiring(kinds: impl IntoIterator<Item = u16>) -> Self {
        Self {
            required_kinds: kinds.into_iter().map(Kind::from).collect(),
            recipient_only: false,
        }
    }

    /// This policy, with gift wraps served only to the keys they address.
    pub(crate) fn recipient_only(mut self, on: bool) -> Self {
        self.recipient_only = on;
        self
    }

    /// Whether gift wraps are served only to the keys they address.
    pub(crate) fn is_recipient_only(&self) -> bool {
        self.recipient_only
    }

    /// Whether `event` may be shown to a connection that has proven `keys`,
    /// as far as this policy goes: a gift wrap needs one of its recipients.
    pub(crate) fn may_read(&self, event: &Event, keys: &HashSet<PublicKey>) -> bool {
        if !self.recipient_only || event.kind != Kind::GiftWrap {
            return true;
        }
        event
            .tags
            .public_keys()
            .any(|recipient| keys.contains(&recipient))
    }

    /// Whether a connection that has not authenticated may not be answered
    /// `filter`: it names no kind, so it may return a restricted one, or it
    /// names one.
    pub(crate) fn restricts(&self, filter: &Filter) -> bool {
        match &filter.kinds {
            None => !self.required_kinds.is_empty(),
            Some(kinds) => kinds.iter().any(|kind| {
                self.required_kinds.contains(kind)
                    || (self.recipient_only && *kind == Kind::GiftWrap)
            }),
        }
    }
}

/// A fresh challenge: 32 random bytes as hex, unguessable and never reused.
pub(crate) fn new_challenge() -> String {
    SecretKey::generate().to_secret_hex()
}

/// The key that signed `event` if it answers `challenge` at `now` (Unix
/// seconds), else why not, worded for the `OK` that tells the client.
pub(crate) fn answer(event: Event, challenge: &str, now: u64) -> Result<PublicKey, &'static str> {
    if event.kind != Kind::Authentication {
        return Err("invalid: the event is not kind 22242");
    }
    if event.created_at.as_secs().abs_diff(now) > TOLERANCE_SECONDS {
        return Err("invalid: created_at is not within ten minutes of now");
    }
    if event.tags.challenge().as_deref() != Some(challenge) {
        return Err("invalid: the challenge is not the one this connection was sent");
    }
    let relay = event
        .tags
        .iter()
        .find(|tag| tag.as_slice().first().map(String::as_str) == Some("relay"));
    if !relay.is_some_and(|tag| tag.content().is_some_and(|url| !url.is_empty())) {
        return Err("invalid: the event names no relay");
    }
    VerifiedEvent::verify(event)
        .map(|verified| verified.event().pubkey)
        .map_err(|_| "invalid: the event's id or signature is wrong")
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;
    use nostr::types::Timestamp;

    use super::*;

    const NOW: u64 = 1_700_000_000;

    fn auth(kind: u16, at: u64, tags: &[&[&str]]) -> (Keys, Event) {
        let keys = Keys::generate();
        let tags = tags
            .iter()
            .map(|tag| Tag::parse(tag.iter().copied()).expect("a tag parses"));
        let event = EventBuilder::new(Kind::from(kind), "")
            .tags(tags)
            .custom_created_at(Timestamp::from(at))
            .finalize(&keys)
            .expect("a generated key signs");
        (keys, event)
    }

    fn valid(challenge: &str) -> (Keys, Event) {
        auth(
            22242,
            NOW,
            &[&["relay", "ws://relay.example"], &["challenge", challenge]],
        )
    }

    fn filter(json: &str) -> Filter {
        serde_json::from_str(json).expect("a filter")
    }

    #[test]
    fn challenges_are_random_and_never_repeat() {
        let (first, second) = (new_challenge(), new_challenge());
        assert_eq!(first.len(), 64);
        assert_ne!(first, second);
    }

    #[test]
    fn an_event_that_answers_the_challenge_authenticates_its_signer() {
        let (keys, event) = valid("c1");
        assert_eq!(answer(event, "c1", NOW), Ok(keys.public_key()));
    }

    #[test]
    fn a_time_within_ten_minutes_either_way_is_fresh_and_beyond_it_is_not() {
        for (at, fresh) in [
            (NOW - 600, true),
            (NOW + 600, true),
            (NOW - 601, false),
            (NOW + 601, false),
        ] {
            let (_, event) = auth(22242, at, &[&["relay", "ws://r"], &["challenge", "c1"]]);
            assert_eq!(answer(event, "c1", NOW).is_ok(), fresh, "{at}");
        }
    }

    #[test]
    fn an_event_that_is_wrong_in_any_one_way_is_refused() {
        let (_, other_kind) = auth(1, NOW, &[&["relay", "ws://r"], &["challenge", "c1"]]);
        let (_, other_challenge) = valid("c2");
        let (_, no_challenge) = auth(22242, NOW, &[&["relay", "ws://r"]]);
        let (_, no_relay) = auth(22242, NOW, &[&["challenge", "c1"]]);
        let (_, mut forged) = valid("c1");
        forged.sig = valid("c1").1.sig;
        for event in [other_kind, other_challenge, no_challenge, no_relay, forged] {
            assert!(answer(event.clone(), "c1", NOW).is_err(), "{event:?}");
        }
    }

    #[test]
    fn with_no_kinds_chosen_nothing_is_restricted() {
        let policy = AuthPolicy::default();
        assert!(!policy.restricts(&filter("{}")));
        assert!(!policy.restricts(&filter(r#"{"kinds":[4]}"#)));
    }

    #[test]
    fn recipient_only_restricts_a_filter_naming_1059_but_not_one_naming_no_kind() {
        let policy = AuthPolicy::default().recipient_only(true);
        assert!(policy.restricts(&filter(r#"{"kinds":[1059]}"#)));
        assert!(!policy.restricts(&filter("{}")));
        assert!(!policy.restricts(&filter(r#"{"kinds":[1]}"#)));
    }

    #[test]
    fn a_wrap_is_readable_by_any_key_its_p_tags_name_when_recipient_only() {
        let (a, b) = (Keys::generate(), Keys::generate());
        let wrap = EventBuilder::new(Kind::GiftWrap, "")
            .tags([Tag::public_key(a.public_key())])
            .finalize(&Keys::generate())
            .expect("signs");
        let note = EventBuilder::new(Kind::TextNote, "")
            .finalize(&Keys::generate())
            .expect("signs");
        let on = AuthPolicy::default().recipient_only(true);
        let keys = |ks: &[&Keys]| ks.iter().map(|k| k.public_key()).collect::<HashSet<_>>();
        assert!(on.may_read(&wrap, &keys(&[&b, &a])));
        assert!(!on.may_read(&wrap, &keys(&[&b])));
        assert!(!on.may_read(&wrap, &keys(&[])));
        assert!(on.may_read(&note, &keys(&[])));
        assert!(AuthPolicy::default().may_read(&wrap, &keys(&[])));
    }

    #[test]
    fn a_filter_is_restricted_when_it_names_a_chosen_kind_or_no_kind() {
        let policy = AuthPolicy::requiring([4, 1059]);
        assert!(policy.restricts(&filter("{}")));
        assert!(policy.restricts(&filter(r#"{"kinds":[4]}"#)));
        assert!(policy.restricts(&filter(r#"{"kinds":[1,1059]}"#)));
        assert!(!policy.restricts(&filter(r#"{"kinds":[1,7]}"#)));
        assert!(!policy.restricts(&filter(r#"{"kinds":[]}"#)));
    }
}
