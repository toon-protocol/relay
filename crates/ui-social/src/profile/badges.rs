//! NIP-58 badges a key shows on its profile: its profile badges list
//! (kind 10008, or the older kind 30008 with `d` = `profile_badges`) names
//! pairs of a badge definition (`a`, kind 30009) and the award (`e`, kind 8)
//! that gave it. A pair is shown only when the award was signed by the
//! definition's issuer and names both the definition and this key, so a
//! key cannot show a badge nobody gave it. Pure functions, no I/O.

use nostr::event::{Event, EventId, Kind};
use nostr::key::PublicKey;
use ui_nip34::repo::{d_tag, first_tag, newest};

/// NIP-58 badge award.
pub(crate) const AWARD: u16 = 8;
/// NIP-58 profile badges.
pub(crate) const PROFILE_BADGES: u16 = 10_008;
/// NIP-58 profile badges, as first specified: a set with `d` =
/// `profile_badges`.
pub(crate) const LEGACY_PROFILE_BADGES: u16 = 30_008;
/// NIP-58 badge definition.
pub(crate) const DEFINITION: u16 = 30_009;

/// A badge a key shows, as its definition describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Badge {
    pub(crate) issuer: PublicKey,
    pub(crate) name: String,
    pub(crate) description: Option<String>,
    pub(crate) awarded_at: u64,
}

/// The `(definition coordinate, award id)` pairs of `key`'s newest profile
/// badges list among `events`, in order. A pure function.
pub(crate) fn accepted(key: &PublicKey, events: &[Event]) -> Vec<(String, EventId)> {
    let Some(list) = newest(events.iter().filter(|event| {
        event.pubkey == *key
            && (event.kind == Kind::from(PROFILE_BADGES)
                || (event.kind == Kind::from(LEGACY_PROFILE_BADGES)
                    && d_tag(event) == "profile_badges"))
    })) else {
        return Vec::new();
    };
    let mut pairs = Vec::new();
    let mut pending: Option<String> = None;
    for tag in list.tags.iter().map(|tag| tag.as_slice()) {
        match tag {
            [name, coordinate, ..] if name == "a" => pending = Some(coordinate.clone()),
            [name, id, ..] if name == "e" => {
                if let (Some(coordinate), Ok(id)) = (pending.take(), EventId::from_hex(id)) {
                    pairs.push((coordinate, id));
                }
            }
            _ => {}
        }
    }
    pairs
}

/// The badges `key` shows: each accepted pair whose award and definition
/// are among `awards` and `definitions`, and agree. A pure function.
pub(crate) fn shown(
    key: &PublicKey,
    pairs: &[(String, EventId)],
    awards: &[Event],
    definitions: &[Event],
) -> Vec<Badge> {
    let key_hex = key.to_hex();
    let mut badges = Vec::new();
    for (coordinate, award_id) in pairs {
        let mut parts = coordinate.splitn(3, ':');
        let (Some(kind), Some(issuer), Some(d)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        let (Ok(DEFINITION), Ok(issuer)) = (kind.parse::<u16>(), PublicKey::from_hex(issuer))
        else {
            continue;
        };
        let names = |event: &Event, name: &str, value: &str| {
            event
                .tags
                .iter()
                .map(|tag| tag.as_slice())
                .any(|tag| matches!(tag, [n, v, ..] if n == name && v == value))
        };
        let Some(award) = awards.iter().find(|award| {
            award.id == *award_id
                && award.kind == Kind::from(AWARD)
                && award.pubkey == issuer
                && names(award, "a", coordinate)
                && names(award, "p", &key_hex)
        }) else {
            continue;
        };
        let Some(definition) = newest(definitions.iter().filter(|definition| {
            definition.kind == Kind::from(DEFINITION)
                && definition.pubkey == issuer
                && d_tag(definition) == d
        })) else {
            continue;
        };
        if badges
            .iter()
            .any(|held: &Badge| held.issuer == issuer && held.name == badge_name(definition))
        {
            continue;
        }
        badges.push(Badge {
            issuer,
            name: badge_name(definition),
            description: first_tag(definition, "description")
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned),
            awarded_at: award.created_at.as_secs(),
        });
    }
    badges
}

/// A definition's `name`, else its `d`.
fn badge_name(definition: &Event) -> String {
    first_tag(definition, "name")
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| d_tag(definition))
        .to_owned()
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;
    use nostr::types::Timestamp;

    use super::*;

    fn signed(keys: &Keys, kind: u16, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .custom_created_at(Timestamp::from(5))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn a_badge_is_shown_only_when_its_issuer_awarded_it_to_this_key() {
        let (issuer, holder, forger) = (Keys::generate(), Keys::generate(), Keys::generate());
        let coordinate = format!("30009:{}:first-merge", issuer.public_key().to_hex());
        let holder_hex = holder.public_key().to_hex();
        let definition = signed(
            &issuer,
            DEFINITION,
            &[
                &["d", "first-merge"],
                &["name", "First merged PR"],
                &["description", "Merged a pull request to gitsail"],
            ],
        );
        let award = signed(&issuer, AWARD, &[&["a", &coordinate], &["p", &holder_hex]]);
        let forged = signed(&forger, AWARD, &[&["a", &coordinate], &["p", &holder_hex]]);
        let elsewhere = signed(
            &issuer,
            AWARD,
            &[&["a", &coordinate], &["p", &forger.public_key().to_hex()]],
        );
        let list = signed(
            &holder,
            PROFILE_BADGES,
            &[
                &["a", &coordinate],
                &["e", &award.id.to_hex()],
                &["a", &coordinate],
                &["e", &forged.id.to_hex()],
                &["a", &coordinate],
                &["e", &elsewhere.id.to_hex()],
                &["e", "f".repeat(64).as_str()],
            ],
        );
        let pairs = accepted(&holder.public_key(), std::slice::from_ref(&list));
        assert_eq!(
            pairs.len(),
            3,
            "an e with no a before it pairs with nothing"
        );
        let badges = shown(
            &holder.public_key(),
            &pairs,
            &[award, forged, elsewhere],
            &[definition],
        );
        assert_eq!(badges.len(), 1, "{badges:?}");
        assert_eq!(badges[0].name, "First merged PR");
        assert_eq!(badges[0].issuer, issuer.public_key());
        assert!(accepted(&forger.public_key(), &[list]).is_empty());
    }
}
