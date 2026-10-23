//! NIP-58 badges as an event's prizes: which badge definitions (kind 30009)
//! belong to a calendar event, and who its hosts awarded them to (kind 8).
//!
//! NIP-58 ties a badge to its issuer and nothing else, so tying one to an
//! event is this page's convention (`docs/apps/social.md`): a badge is the
//! event's when a host defined it and either the definition `a`-tags the
//! calendar event or the calendar event `a`-tags the definition. An award
//! counts only when the definition's own author signed it, as NIP-58 has
//! the issuer award its badges. Pure functions over events, with no I/O.

use std::collections::HashMap;

use nostr::event::{Event, Kind};
use nostr::key::PublicKey;
use ui_nip34::repo::{d_tag, first_tag};

use crate::media::Media;

/// NIP-58 badge definition.
pub(crate) const DEFINITION: u16 = 30_009;
/// NIP-58 badge award.
pub(crate) const AWARD: u16 = 8;

/// A badge definition, read.
#[derive(Debug, Clone)]
pub(crate) struct Badge {
    author: PublicKey,
    d: String,
    name: String,
    description: Option<String>,
    image: Option<Media>,
}

impl Badge {
    /// `event` as a badge definition, if it is a 30009 with a `d`.
    pub(crate) fn read(event: &Event) -> Option<Self> {
        if event.kind != Kind::from(DEFINITION) || d_tag(event).is_empty() {
            return None;
        }
        let d = d_tag(event).to_owned();
        let name = first_tag(event, "name")
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .map_or_else(|| d.clone(), str::to_owned);
        let image = ui_nip34::repo::tag_values(event, "thumb")
            .first()
            .or_else(|| ui_nip34::repo::tag_values(event, "image").first())
            .and_then(|url| Media::new(url, Some("image/*")));
        Some(Self {
            author: event.pubkey,
            d,
            name,
            description: first_tag(event, "description")
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned),
            image,
        })
    }

    pub(crate) fn coordinate(&self) -> String {
        format!("{DEFINITION}:{}:{}", self.author.to_hex(), self.d)
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// The badge's image, as the browser draws it (never fetched here).
    pub(crate) fn image_html(&self) -> Option<String> {
        self.image.as_ref().map(|image| image.html("badge-img"))
    }
}

/// Whether `event` names `coordinate` in an `a` tag.
fn a_tags(event: &Event, coordinate: &str) -> bool {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .any(|tag| matches!(tag, [name, value, ..] if name == "a" && value == coordinate))
}

/// The event's badges among `definitions`: one by a host in `hosts` that
/// `a`-tags `coordinate` (the calendar event's) or that `calendar` itself
/// `a`-tags. Newest definition of each, in name order. A
/// pure function.
pub(crate) fn event_badges(
    definitions: &[Event],
    calendar: &Event,
    coordinate: &str,
    hosts: &[PublicKey],
) -> Vec<Badge> {
    let mut held: HashMap<(PublicKey, String), &Event> = HashMap::new();
    for event in definitions {
        if event.kind != Kind::from(DEFINITION) || !hosts.contains(&event.pubkey) {
            continue;
        }
        let key = (event.pubkey, d_tag(event).to_owned());
        match held.get(&key) {
            Some(current) if !ui_nip34::repo::supersedes(event, current) => {}
            _ => {
                held.insert(key, event);
            }
        }
    }
    let mut badges: Vec<Badge> = held
        .into_values()
        .filter_map(Badge::read)
        .filter(|badge| {
            let definition_names_event = definitions.iter().any(|event| {
                event.pubkey == badge.author && d_tag(event) == badge.d && a_tags(event, coordinate)
            });
            definition_names_event || a_tags(calendar, &badge.coordinate())
        })
        .collect();
    badges.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.d.cmp(&b.d)));
    badges
}

/// Who holds which of `badges`, from `awards`: for each key a kind 8 award
/// names in a `p` tag, the indexes of the badges awarded, once each, in
/// badge order. An award counts only when the badge's author signed it. A
/// pure function.
pub(crate) fn holders(awards: &[Event], badges: &[Badge]) -> HashMap<PublicKey, Vec<usize>> {
    let mut held: HashMap<PublicKey, Vec<usize>> = HashMap::new();
    for award in awards {
        if award.kind != Kind::from(AWARD) {
            continue;
        }
        for (index, badge) in badges.iter().enumerate() {
            if award.pubkey != badge.author || !a_tags(award, &badge.coordinate()) {
                continue;
            }
            for tag in award.tags.iter().map(|tag| tag.as_slice()) {
                if let [name, key, ..] = tag
                    && name == "p"
                    && let Ok(key) = PublicKey::from_hex(key)
                {
                    let badges = held.entry(key).or_default();
                    if !badges.contains(&index) {
                        badges.push(index);
                    }
                }
            }
        }
    }
    for badges in held.values_mut() {
        badges.sort_unstable();
    }
    held
}

/// How many distinct keys hold each of `badges`. A pure function.
pub(crate) fn counts(holders: &HashMap<PublicKey, Vec<usize>>, badges: usize) -> Vec<usize> {
    let mut counts = vec![0; badges];
    for held in holders.values() {
        for index in held {
            if let Some(count) = counts.get_mut(*index) {
                *count += 1;
            }
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;
    use nostr::types::Timestamp;

    use super::*;

    fn signed(keys: &Keys, kind: u16, at: u64, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .custom_created_at(Timestamp::from(at))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn a_badge_is_the_events_when_a_host_ties_it_either_way() {
        let (host, stranger) = (Keys::generate(), Keys::generate());
        let host_hex = host.public_key().to_hex();
        let coordinate = format!("31923:{host_hex}:jam");
        let crowd = format!("30009:{host_hex}:crowd");
        let calendar = signed(&host, 31_923, 1, &[&["d", "jam"], &["a", &crowd]]);
        let definitions = [
            signed(
                &host,
                DEFINITION,
                1,
                &[&["d", "winner"], &["name", "Winner"], &["a", &coordinate]],
            ),
            signed(
                &host,
                DEFINITION,
                1,
                &[&["d", "crowd"], &["name", "Crowd pick"]],
            ),
            signed(
                &host,
                DEFINITION,
                1,
                &[&["d", "other"], &["name", "Unrelated"]],
            ),
            signed(
                &stranger,
                DEFINITION,
                1,
                &[&["d", "fake"], &["a", &coordinate]],
            ),
        ];
        let badges = event_badges(&definitions, &calendar, &coordinate, &[host.public_key()]);
        let names: Vec<&str> = badges.iter().map(Badge::name).collect();
        assert_eq!(names, ["Crowd pick", "Winner"]);
    }

    #[test]
    fn only_the_issuers_award_counts_and_each_holder_once() {
        let (host, faker, maya, leo) = (
            Keys::generate(),
            Keys::generate(),
            Keys::generate(),
            Keys::generate(),
        );
        let badge = Badge::read(&signed(&host, DEFINITION, 1, &[&["d", "winner"]])).unwrap();
        let a = badge.coordinate();
        let (maya_hex, leo_hex) = (maya.public_key().to_hex(), leo.public_key().to_hex());
        let awards = [
            signed(&host, AWARD, 2, &[&["a", &a], &["p", &maya_hex]]),
            signed(
                &host,
                AWARD,
                3,
                &[&["a", &a], &["p", &maya_hex], &["p", "nope"]],
            ),
            signed(&faker, AWARD, 4, &[&["a", &a], &["p", &leo_hex]]),
        ];
        let held = holders(&awards, std::slice::from_ref(&badge));
        assert_eq!(held.get(&maya.public_key()), Some(&vec![0]));
        assert_eq!(
            held.get(&leo.public_key()),
            None,
            "a stranger cannot award the host's badge"
        );
        assert_eq!(counts(&held, 1), vec![1]);
    }
}
