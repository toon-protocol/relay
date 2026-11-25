//! Event reading: a stored Nostr event read the way NIP-01 says. Its tags,
//! which version of a replaceable or addressable event is current (ADR 0002),
//! and its coordinate. Pure functions over events, with no store access, so
//! every app UI reads the same event the same way.

use std::fmt;

use nostr::event::Event;
use nostr::key::PublicKey;

/// The values of the first tag called `name`, after the name.
pub fn first_tag_values<'a>(event: &'a Event, name: &str) -> &'a [String] {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find(|tag| tag.first().is_some_and(|first| first == name))
        .map_or(&[], |tag| &tag[1..])
}

/// The first value of the first tag called `name`.
pub fn first_tag_value<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    first_tag_values(event, name).first().map(String::as_str)
}

/// The first value of every tag called `name`, in order.
pub fn every_tag_value<'a>(event: &'a Event, name: &str) -> Vec<&'a str> {
    event
        .tags
        .iter()
        .filter_map(|tag| match tag.as_slice() {
            [tag_name, value, ..] if tag_name == name => Some(value.as_str()),
            _ => None,
        })
        .collect()
}

/// The event's `d` tag, or empty.
pub fn d_tag(event: &Event) -> &str {
    first_tag_value(event, "d").unwrap_or("")
}

/// Whether `candidate` is a newer replacement than `held`: later, and on a
/// `created_at` tie the lowest event id (ADR 0002).
pub fn supersedes(candidate: &Event, held: &Event) -> bool {
    (candidate.created_at, held.id) > (held.created_at, candidate.id)
}

/// The newest of `events`, or none.
pub fn newest<'a>(events: impl IntoIterator<Item = &'a Event>) -> Option<&'a Event> {
    events
        .into_iter()
        .fold(None, |held: Option<&Event>, candidate| match held {
            Some(held) if !supersedes(candidate, held) => Some(held),
            _ => Some(candidate),
        })
}

/// An addressable coordinate, `<kind>:<pubkey hex>:<d>`: the slot a
/// replaceable or addressable event fills. `d` is everything after the second
/// `:`, so it may itself contain `:`, and it may be empty (a replaceable kind).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Address {
    /// The event kind.
    pub kind: u16,
    /// The author.
    pub pubkey: PublicKey,
    /// The `d` tag.
    pub d: String,
}

impl Address {
    /// The coordinate a string spells, if its kind and pubkey are valid.
    pub fn parse(coordinate: &str) -> Option<Self> {
        let mut parts = coordinate.splitn(3, ':');
        let (kind, pubkey, d) = (parts.next()?, parts.next()?, parts.next()?);
        Some(Self {
            kind: kind.parse().ok()?,
            pubkey: PublicKey::from_hex(pubkey).ok()?,
            d: d.to_owned(),
        })
    }

    /// The coordinate of `event`: its kind, author and `d` tag.
    pub fn of(event: &Event) -> Self {
        Self {
            kind: event.kind.as_u16(),
            pubkey: event.pubkey,
            d: d_tag(event).to_owned(),
        }
    }
}

impl fmt::Display for Address {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}:{}:{}",
            self.kind,
            self.pubkey.to_hex(),
            self.d
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;
    use nostr::types::Timestamp;

    fn event(keys: &Keys, kind: u16, at: u64, tag: &str) -> Event {
        EventBuilder::new(nostr::event::Kind::from(kind), tag)
            .tags([Tag::parse(["d", "x"]).unwrap()])
            .custom_created_at(Timestamp::from_secs(at))
            .finalize(keys)
            .unwrap()
    }

    /// The store's rule (`store.rs`), as it states it: the newer wins, and on
    /// a tie the lower id.
    fn store_keeps(events: &[Event]) -> Option<&Event> {
        let mut kept: Option<&Event> = None;
        for event in events {
            let held_wins = kept.is_some_and(|held| match held.created_at.cmp(&event.created_at) {
                std::cmp::Ordering::Greater => true,
                std::cmp::Ordering::Equal => held.id < event.id,
                std::cmp::Ordering::Less => false,
            });
            if !held_wins {
                kept = Some(event);
            }
        }
        kept
    }

    #[test]
    fn newest_is_the_one_the_store_keeps_for_any_set_in_any_order() {
        let keys = Keys::generate();
        // Few distinct times, so ties are common; every rotation and the
        // reverse of each generated set is checked.
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        for round in 0..200 {
            let mut events = Vec::new();
            for n in 0..(1 + round % 6) {
                seed = seed.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1);
                events.push(event(
                    &keys,
                    30_000,
                    100 + (seed >> 60) % 3,
                    &format!("{round}-{n}"),
                ));
            }
            for turn in 0..events.len() {
                events.rotate_left(1);
                let expected = store_keeps(&events).map(|e| e.id);
                assert_eq!(newest(&events).map(|e| e.id), expected, "turn {turn}");
                let reversed: Vec<Event> = events.iter().rev().cloned().collect();
                assert_eq!(newest(&reversed).map(|e| e.id), expected);
            }
        }
        assert!(newest(&[]).is_none());
    }

    #[test]
    fn a_tie_goes_to_the_lowest_id() {
        let keys = Keys::generate();
        let (a, b) = (event(&keys, 30_000, 5, "a"), event(&keys, 30_000, 5, "b"));
        let lowest = if a.id < b.id { &a } else { &b };
        assert_eq!(newest([&a, &b]).map(|e| e.id), Some(lowest.id));
        assert_eq!(newest([&b, &a]).map(|e| e.id), Some(lowest.id));
        assert!(!supersedes(&a, &a));
    }

    #[test]
    fn a_coordinate_round_trips_with_a_colon_in_d() {
        let key = Keys::generate().public_key();
        for d in ["", "plain", "a:b", "::", "x:y:z"] {
            let address = Address {
                kind: 30_023,
                pubkey: key,
                d: d.to_owned(),
            };
            assert_eq!(Address::parse(&address.to_string()), Some(address), "{d:?}");
        }
        assert_eq!(Address::parse("30023:nothex:d"), None);
        assert_eq!(Address::parse("x:".to_owned().as_str()), None);
        assert_eq!(Address::parse(&format!("30023:{}", key.to_hex())), None);
    }

    #[test]
    fn tags_are_read_by_what_they_return() {
        let keys = Keys::generate();
        let event = EventBuilder::new(nostr::event::Kind::from(1), "")
            .tags([
                Tag::parse(["t", "a", "b"]).unwrap(),
                Tag::parse(["t", "c"]).unwrap(),
                Tag::parse(["d", "id"]).unwrap(),
            ])
            .finalize(&keys)
            .unwrap();
        assert_eq!(first_tag_values(&event, "t"), ["a", "b"]);
        assert_eq!(first_tag_value(&event, "t"), Some("a"));
        assert_eq!(every_tag_value(&event, "t"), ["a", "c"]);
        assert_eq!(d_tag(&event), "id");
        assert_eq!(first_tag_value(&event, "nope"), None);
    }
}
