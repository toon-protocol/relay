//! Tips (proposal 16 of `docs/social-for-builders-ux.md`). The pages offer no
//! way to send one (app UIs on the relay are read-only, ADR 0001), but they
//! show the tips a recipient acknowledged: [`tally`] is the reader rule of
//! ADR 0007 over the tip NIP's acknowledgements (kind 9851), a pure function.
//!
//! The Nostr shape for "who gets a tip" is NIP-57's `zap` tag (Appendix G):
//! `["zap", <pubkey>, <relay>, <weight>]` on any event. Zaps themselves are
//! out on TOON (the receipt is written by a Lightning server, not by the
//! payer, and nothing here carries Lightning), and the relay must not check
//! a payment: that is the connector's alone. So a page shows a split the
//! owner wrote as text, "Tip split (set by the owner)", and nothing that
//! looks like a button. `docs/apps/social.md` says how the pages draw tips.

use std::collections::{HashMap, HashSet};

use nostr::event::{Event, EventId, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use relay_ui::ReadOnlyStore;
use relay_ui::chrome::{escape, name};
use ui_nip34::card::plural;
use ui_nip34::repo::first_tag;

use crate::kinds::{TIP_ACKNOWLEDGEMENT, TIP_ROUTE};

/// The most acknowledgements one page reads.
const LIMIT: usize = 5_000;

/// An amount's unit: the asset code and its scale. Two amounts are in the
/// same unit only when both are equal; nothing converts between units.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub(crate) struct Unit {
    pub(crate) code: String,
    pub(crate) scale: u32,
}

impl Unit {
    /// `base` units of this unit as a decimal in major units: `2500000`
    /// at scale 6 is `2.5 USDC`.
    pub(crate) fn show(&self, base: u128) -> String {
        let scale = self.scale as usize;
        let digits = format!("{base:0>width$}", width = scale + 1);
        let (whole, fraction) = digits.split_at(digits.len() - scale);
        let fraction = fraction.trim_end_matches('0');
        if fraction.is_empty() {
            format!("{whole} {}", self.code)
        } else {
            format!("{whole}.{fraction} {}", self.code)
        }
    }
}

/// An event's whole `amount` tag, `["amount", <base units>, <code>, <scale>]`.
pub(crate) fn amount_of(event: &Event) -> Option<(u128, Unit)> {
    event.tags.iter().find_map(|tag| match tag.as_slice() {
        [name, base, code, scale, ..] if name == "amount" => {
            let code = code.trim();
            if code.is_empty() {
                return None;
            }
            Some((
                base.trim().parse().ok()?,
                Unit {
                    code: code.to_owned(),
                    // A scale beyond what a u128 can hold is not a unit.
                    scale: scale.trim().parse().ok().filter(|scale| *scale <= 38)?,
                },
            ))
        }
        _ => None,
    })
}

/// What a tip was for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Subject {
    /// The person: an acknowledgement with no `e` and no `a` tag.
    Person,
    /// A note or a goal, by an `e` tag.
    Event(EventId),
    /// A repo or a bounty, by an `a` tag's coordinate.
    Address(String),
}

impl Subject {
    fn is_tipped_by(&self, ack: &Event) -> bool {
        let mut tags = ack.tags.iter().map(|tag| tag.as_slice());
        match self {
            Self::Person => {
                !tags.any(|tag| matches!(tag, [name, ..] if name == "e" || name == "a"))
            }
            Self::Event(id) => {
                let id = id.to_hex();
                tags.any(|tag| matches!(tag, [name, value, ..] if name == "e" && *value == id))
            }
            Self::Address(coordinate) => tags
                .any(|tag| matches!(tag, [name, value, ..] if name == "a" && value == coordinate)),
        }
    }
}

/// What a recipient acknowledged for one subject: the arrived amounts per
/// unit, how many tips counted, and how many distinct payers the
/// acknowledgements name.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Totals {
    pub(crate) per_unit: Vec<(Unit, u128)>,
    pub(crate) payers: usize,
    pub(crate) tips: usize,
}

impl Totals {
    /// The total in `unit`, zero when nothing arrived in it.
    pub(crate) fn in_unit(&self, unit: &Unit) -> u128 {
        self.per_unit
            .iter()
            .find(|(each, _)| each == unit)
            .map_or(0, |(_, total)| *total)
    }

    /// Whether no tip counted.
    pub(crate) fn is_empty(&self) -> bool {
        self.tips == 0
    }
}

/// The tips ADR 0007 counts for `subject` out of `events`: only kind 9851
/// signed by `recipient`; the amount that arrived, summed per unit and never
/// converted; one tip per `receipt` (the earliest whole acknowledgement of
/// it, ties by id); payers counted only where named by a `p` tag. Anything else
/// in `events` changes no figure. A pure function.
pub(crate) fn tally(events: &[Event], recipient: &PublicKey, subject: &Subject) -> Totals {
    let mut acks: Vec<(&Event, &str, (u128, Unit))> = events
        .iter()
        .filter(|ack| ack.kind == Kind::from(TIP_ACKNOWLEDGEMENT) && ack.pubkey == *recipient)
        .filter(|ack| subject.is_tipped_by(ack))
        .filter_map(|ack| {
            let receipt = first_tag(ack, "receipt").filter(|receipt| !receipt.is_empty())?;
            Some((ack, receipt, amount_of(ack)?))
        })
        .collect();
    acks.sort_by_key(|(a, _, _)| (a.created_at, a.id));
    let mut receipts: HashSet<&str> = HashSet::new();
    let mut payers: HashSet<PublicKey> = HashSet::new();
    let mut per_unit: Vec<(Unit, u128)> = Vec::new();
    let mut tips = 0;
    for (ack, receipt, (base, unit)) in acks {
        if !receipts.insert(receipt) {
            continue;
        }
        tips += 1;
        match per_unit.iter_mut().find(|(each, _)| *each == unit) {
            Some((_, total)) => *total = total.saturating_add(base),
            None => per_unit.push((unit, base)),
        }
        payers.extend(first_tag(ack, "p").and_then(|key| PublicKey::from_hex(key).ok()));
    }
    per_unit.sort();
    Totals {
        per_unit,
        payers: payers.len(),
        tips,
    }
}

/// The tips of `subject` that `recipient` acknowledged, as this relay's
/// store holds them (it reads nothing else).
pub(crate) async fn totals(
    store: &ReadOnlyStore,
    recipient: &PublicKey,
    subject: &Subject,
) -> Totals {
    let filter = Filter::new()
        .kind(Kind::from(TIP_ACKNOWLEDGEMENT))
        .author(*recipient)
        .limit(LIMIT);
    let filter = match subject {
        Subject::Person => filter,
        Subject::Event(id) => filter.event(*id),
        Subject::Address(coordinate) => {
            filter.custom_tag(SingleLetterTag::LOWERCASE_A, coordinate.as_str())
        }
    };
    tally(&store.query(filter).await, recipient, subject)
}

/// Whether `key` can be tipped: its newest tip route (kind 10850) names a
/// connector, an ILP address and a sealing key. Read-only; the route's
/// details are the payer's to read from the connector.
pub(crate) async fn route_card(store: &ReadOnlyStore, key: &PublicKey) -> String {
    let routes = store
        .query(
            Filter::new()
                .kind(Kind::from(TIP_ROUTE))
                .author(*key)
                .limit(1),
        )
        .await;
    let tagged = |event: &Event, wanted: &str| {
        event.tags.iter().map(|tag| tag.as_slice()).any(
            |tag| matches!(tag, [name, value, ..] if name == wanted && !value.trim().is_empty()),
        )
    };
    let Some(route) = routes.first() else {
        return String::new();
    };
    if !["connector_url", "ilp_address", "connector_seal_key"]
        .iter()
        .all(|wanted| tagged(route, wanted))
    {
        return String::new();
    }
    "<div class=\"side-card\" data-tip-route><h2>Can be tipped</h2><p class=\"muted small\">This \
     person has published a tip route. This page cannot send a tip: a client does, with a \
     payment.</p></div>"
        .to_owned()
}

/// A card with the tips received, or nothing when none counted.
pub(crate) fn received_card(totals: &Totals) -> String {
    if totals.is_empty() {
        return String::new();
    }
    let mut out =
        String::from("<div class=\"side-card\" data-tips-received><h2>Tips received</h2><ul>");
    for (unit, total) in &totals.per_unit {
        out.push_str(&format!(
            "<li><span class=\"strong\" data-tip-total>{}</span></li>",
            escape(&unit.show(*total))
        ));
    }
    out.push_str(&format!(
        "</ul><p class=\"muted small\" data-tip-payers=\"{}\">{} named {}</p>\
         <p class=\"muted small\">What arrived, as the recipient acknowledged it. Amounts in \
         different units are not added.</p></div>",
        totals.payers,
        totals.payers,
        plural(totals.payers, "payer", "payers"),
    ));
    out
}

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
    use nostr::types::Timestamp;

    use super::*;

    fn with(tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(30_617), "")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .finalize(&Keys::generate())
            .unwrap()
    }

    fn ack(keys: &Keys, kind: u16, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .finalize(keys)
            .unwrap()
    }

    fn goal_id() -> EventId {
        EventId::from_hex(&"ab".repeat(32)).unwrap()
    }

    #[test]
    fn arrived_amounts_sum_per_unit_and_are_never_converted() {
        let owner = Keys::generate();
        let goal = goal_id().to_hex();
        let events = [
            ack(
                &owner,
                9851,
                &[
                    &["e", &goal],
                    &["amount", "2500000", "USDC", "6"],
                    &["receipt", "r1"],
                ],
            ),
            ack(
                &owner,
                9851,
                &[
                    &["e", &goal],
                    &["amount", "500000", "USDC", "6"],
                    &["receipt", "r2"],
                ],
            ),
            ack(
                &owner,
                9851,
                &[
                    &["e", &goal],
                    &["amount", "9", "EUR", "2"],
                    &["receipt", "r3"],
                ],
            ),
            ack(
                &owner,
                9851,
                &[
                    &["e", &goal],
                    &["amount", "7", "USDC", "2"],
                    &["receipt", "r4"],
                ],
            ),
        ];
        let totals = tally(&events, &owner.public_key(), &Subject::Event(goal_id()));
        let usdc = Unit {
            code: "USDC".into(),
            scale: 6,
        };
        assert_eq!(totals.in_unit(&usdc), 3_000_000);
        assert_eq!(totals.per_unit.len(), 3);
        assert_eq!(usdc.show(3_000_000), "3 USDC");
        assert_eq!(usdc.show(2_500_000), "2.5 USDC");
        assert_eq!(usdc.show(5), "0.000005 USDC");
    }

    #[test]
    fn only_the_recipients_acknowledgements_count() {
        let owner = Keys::generate();
        let stranger = Keys::generate();
        let goal = goal_id().to_hex();
        let tags: &[&[&str]] = &[
            &["e", &goal],
            &["amount", "100", "USDC", "6"],
            &["receipt", "r1"],
        ];
        let events = [
            ack(&stranger, 9851, tags),
            // A tip receipt (9850) found on the relay is not counted.
            ack(&owner, 9850, tags),
            // Nor is a zap receipt.
            ack(&owner, 9735, tags),
        ];
        let totals = tally(&events, &owner.public_key(), &Subject::Event(goal_id()));
        assert!(totals.is_empty());
        assert_eq!(totals, Totals::default());
    }

    #[test]
    fn an_unnamed_payer_adds_to_the_total_and_not_the_count() {
        let owner = Keys::generate();
        let payer = Keys::generate().public_key().to_hex();
        let events = [
            ack(
                &owner,
                9851,
                &[
                    &["amount", "100", "USDC", "6"],
                    &["p", &payer],
                    &["receipt", "r1"],
                ],
            ),
            ack(
                &owner,
                9851,
                &[
                    &["amount", "100", "USDC", "6"],
                    &["p", &payer],
                    &["receipt", "r2"],
                ],
            ),
            ack(
                &owner,
                9851,
                &[&["amount", "100", "USDC", "6"], &["receipt", "r3"]],
            ),
        ];
        let totals = tally(&events, &owner.public_key(), &Subject::Person);
        let usdc = Unit {
            code: "USDC".into(),
            scale: 6,
        };
        assert_eq!(
            (totals.in_unit(&usdc), totals.payers, totals.tips),
            (300, 1, 3)
        );
    }

    #[test]
    fn two_acknowledgements_of_one_receipt_are_one_tip() {
        let owner = Keys::generate();
        let tags: &[&[&str]] = &[&["amount", "100", "USDC", "6"], &["receipt", "same"]];
        let events = [
            ack(&owner, 9851, tags),
            ack(
                &owner,
                9851,
                &[
                    &["amount", "100", "USDC", "6"],
                    &["receipt", "same"],
                    &["client", "x"],
                ],
            ),
        ];
        let totals = tally(&events, &owner.public_key(), &Subject::Person);
        assert_eq!(totals.tips, 1);
        assert_eq!(totals.per_unit[0].1, 100);
    }

    #[test]
    fn the_earliest_whole_acknowledgement_of_a_receipt_is_the_one_counted() {
        let owner = Keys::generate();
        let at = |seconds: u64, amount: &[&str]| {
            EventBuilder::new(Kind::from(9851), "")
                .tags([
                    Tag::parse(amount.to_vec()).unwrap(),
                    Tag::parse(["receipt", "same"]).unwrap(),
                ])
                .custom_created_at(Timestamp::from(seconds))
                .finalize(&owner)
                .unwrap()
        };
        let events = [
            at(3, &["amount", "300", "USDC", "6"]),
            at(2, &["amount", "200", "USDC", "6"]),
            // Earliest, but not a whole amount: not a tip, and it shadows none.
            at(1, &["amount", "100"]),
        ];
        let totals = tally(&events, &owner.public_key(), &Subject::Person);
        assert_eq!((totals.tips, totals.per_unit[0].1), (1, 200));
    }

    #[test]
    fn a_tip_counts_only_for_what_it_names() {
        let owner = Keys::generate();
        let coordinate = format!("30617:{}:tidepool", owner.public_key().to_hex());
        let events = [
            ack(
                &owner,
                9851,
                &[
                    &["a", &coordinate],
                    &["amount", "1", "USDC", "6"],
                    &["receipt", "r1"],
                ],
            ),
            ack(
                &owner,
                9851,
                &[&["amount", "1", "USDC", "6"], &["receipt", "r2"]],
            ),
            // Not a whole amount, or no receipt: not a tip.
            ack(
                &owner,
                9851,
                &[&["a", &coordinate], &["amount", "1"], &["receipt", "r3"]],
            ),
            ack(
                &owner,
                9851,
                &[&["a", &coordinate], &["amount", "1", "USDC", "6"]],
            ),
        ];
        let key = owner.public_key();
        assert_eq!(tally(&events, &key, &Subject::Address(coordinate)).tips, 1);
        assert_eq!(tally(&events, &key, &Subject::Person).tips, 1);
        assert_eq!(tally(&events, &key, &Subject::Event(goal_id())).tips, 0);
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
