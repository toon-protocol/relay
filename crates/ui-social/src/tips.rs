//! Tips (proposal 16 of `docs/social-for-builders-ux.md`), which TOON
//! cannot do yet, so the pages offer no way to send one.
//!
//! The Nostr shape for "who gets a tip" is NIP-57's `zap` tag (Appendix G):
//! `["zap", <pubkey>, <relay>, <weight>]` on any event. Zaps themselves are
//! out on TOON (the receipt is written by a Lightning server, not by the
//! payer, and nothing here carries Lightning), and the relay must not check
//! a payment: that is the connector's alone. So a page shows a split the
//! owner wrote as text, "Tip split (set by the owner)", and nothing that
//! looks like a button. `docs/apps/social.md` says what would close the gap.

use std::collections::HashMap;

use nostr::event::Event;
use nostr::key::PublicKey;
use relay_ui::chrome::{escape, name};

/// One receiver of a split, with its share in whole percent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Share {
    pub(crate) key: PublicKey,
    pub(crate) percent: u32,
}

/// The split `event`'s `zap` tags describe, by NIP-57 Appendix G: weights
/// are summed and each receiver gets its part; with no weights at all the
/// receivers split evenly; when only some carry one, a receiver without a
/// weight gets nothing. Each key once (the first tag naming it). A pure
/// function.
pub(crate) fn split(event: &Event) -> Vec<Share> {
    let mut receivers: Vec<(PublicKey, Option<f64>)> = Vec::new();
    for tag in event.tags.iter().map(|tag| tag.as_slice()) {
        let (key, weight) = match tag {
            [name, key] | [name, key, _] if name == "zap" => (key, None),
            [name, key, _, weight, ..] if name == "zap" => (key, Some(weight)),
            _ => continue,
        };
        let Ok(key) = PublicKey::from_hex(key) else {
            continue;
        };
        if receivers.iter().any(|(held, _)| *held == key) {
            continue;
        }
        let weight = weight
            .and_then(|weight| weight.trim().parse::<f64>().ok())
            .filter(|weight| weight.is_finite() && *weight >= 0.0);
        receivers.push((key, weight));
    }
    if receivers.is_empty() {
        return Vec::new();
    }
    let weighted = receivers.iter().any(|(_, weight)| weight.is_some());
    let weights: Vec<f64> = receivers
        .iter()
        .map(|(_, weight)| match (weighted, weight) {
            (false, _) => 1.0,
            (true, Some(weight)) => *weight,
            (true, None) => 0.0,
        })
        .collect();
    let total: f64 = weights.iter().sum();
    if total <= 0.0 {
        return Vec::new();
    }
    receivers
        .into_iter()
        .zip(weights)
        .map(|((key, _), weight)| Share {
            key,
            // A share is 0 to 100 by construction, so the cast is exact.
            percent: (weight / total * 100.0).round().clamp(0.0, 100.0) as u32,
        })
        .collect()
}

/// A sidebar card with the split `event` carries, or nothing. `whose` says
/// whose event it is ("the owner", "the publisher").
pub(crate) fn split_card(event: &Event, names: &HashMap<PublicKey, String>, whose: &str) -> String {
    let shares = split(event);
    if shares.is_empty() {
        return String::new();
    }
    let mut out = format!(
        "<div class=\"side-card\" data-tip-split><h2>Tip split (set by {})</h2><ul class=\"split-list\">",
        escape(whose)
    );
    for share in &shares {
        out.push_str(&format!(
            "<li>{}<span class=\"muted\">{}%</span></li>",
            name(names, &share.key),
            share.percent
        ));
    }
    out.push_str(
        "</ul><p class=\"muted small\">As written in the event's <code>zap</code> tags (NIP-57). \
         This page cannot send a tip: zaps are not used on TOON, and the relay never checks a \
         payment.</p></div>",
    );
    out
}

/// The keys a split names, to load their names.
pub(crate) fn split_keys(event: &Event) -> Vec<PublicKey> {
    split(event).into_iter().map(|share| share.key).collect()
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Kind, Tag};
    use nostr::key::Keys;

    use super::*;

    fn with(tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(30_617), "")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .finalize(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn weights_are_shares_of_their_sum() {
        let (a, b, c) = (
            Keys::generate().public_key(),
            Keys::generate().public_key(),
            Keys::generate().public_key(),
        );
        let event = with(&[
            &["zap", &a.to_hex(), "wss://x", "1"],
            &["zap", &b.to_hex(), "wss://x", "1"],
            &["zap", &c.to_hex(), "wss://x", "2"],
        ]);
        let percents: Vec<u32> = split(&event).iter().map(|share| share.percent).collect();
        assert_eq!(percents, [25, 25, 50]);
    }

    #[test]
    fn no_weights_split_evenly_and_a_missing_weight_gets_nothing() {
        let (a, b) = (Keys::generate().public_key(), Keys::generate().public_key());
        let even = with(&[&["zap", &a.to_hex(), "wss://x"], &["zap", &b.to_hex()]]);
        assert!(split(&even).iter().all(|share| share.percent == 50));
        let partial = with(&[
            &["zap", &a.to_hex(), "wss://x", "3"],
            &["zap", &b.to_hex(), "wss://x"],
        ]);
        let shares = split(&partial);
        assert_eq!((shares[0].percent, shares[1].percent), (100, 0));
        assert!(split(&with(&[&["zap", "nope", "", "1"]])).is_empty());
        assert!(split(&with(&[&["zap", &a.to_hex(), "", "0"]])).is_empty());
        assert!(split(&with(&[])).is_empty());
    }
}
