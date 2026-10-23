//! A key's NIP-51 mute list (kind 10000) as a filter any `/social` list can
//! apply (proposal 18 of `docs/social-for-builders-ux.md`): the feeds and
//! "trending today" drop what the viewer muted.
//!
//! Only the list's public tags are read: `p` (people), `t` (hashtags),
//! `word` (lowercase words) and `e` (threads). Private items are encrypted
//! to the list's author in `.content`, and the relay holds no key that could
//! read them, so they stay muted only in the author's own client. A mute
//! hides; it removes nothing from the store, and someone reading without
//! the lens sees everything.

use std::collections::HashSet;

use nostr::event::{Event, EventId, Kind};
use nostr::filter::Filter;
use nostr::key::PublicKey;
use relay_ui::ReadOnlyStore;
use ui_nip34::card::RepoCard;
use ui_nip34::repo::{RepoAddress, newest, topics};

use crate::kinds::MUTE_LIST;

/// What one key muted, publicly.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Mutes {
    keys: HashSet<PublicKey>,
    hashtags: HashSet<String>,
    words: Vec<String>,
    threads: HashSet<EventId>,
}

impl Mutes {
    /// The newest mute list of `viewer` in the store, or nothing muted.
    pub(crate) async fn load(store: &ReadOnlyStore, viewer: &PublicKey) -> Self {
        let lists = store
            .query(Filter::new().kind(Kind::from(MUTE_LIST)).author(*viewer))
            .await;
        newest(lists.iter().filter(|list| list.pubkey == *viewer))
            .map(Self::from_list)
            .unwrap_or_default()
    }

    /// The public items of a mute list. A pure function.
    pub(crate) fn from_list(list: &Event) -> Self {
        let mut mutes = Self::default();
        for tag in list.tags.iter().map(|tag| tag.as_slice()) {
            let [name, value, ..] = tag else { continue };
            let value = value.trim();
            match name.as_str() {
                "p" => {
                    if let Ok(key) = PublicKey::from_hex(value) {
                        mutes.keys.insert(key);
                    }
                }
                "t" if !value.is_empty() => {
                    mutes
                        .hashtags
                        .insert(value.trim_start_matches('#').to_lowercase());
                }
                "word" if !value.is_empty() => {
                    let word = value.to_lowercase();
                    if !mutes.words.contains(&word) {
                        mutes.words.push(word);
                    }
                }
                "e" => {
                    if let Ok(id) = EventId::from_hex(value) {
                        mutes.threads.insert(id);
                    }
                }
                _ => {}
            }
        }
        mutes
    }

    /// Whether nothing is muted.
    pub(crate) fn is_empty(&self) -> bool {
        self.keys.is_empty()
            && self.hashtags.is_empty()
            && self.words.is_empty()
            && self.threads.is_empty()
    }

    /// How many people and hashtags are muted, for a line that says so.
    pub(crate) fn counts(&self) -> (usize, usize) {
        (self.keys.len(), self.hashtags.len())
    }

    /// Whether `key` is muted.
    pub(crate) fn hides_key(&self, key: &PublicKey) -> bool {
        self.keys.contains(key)
    }

    /// Whether `hashtag` (with or without `#`, any case) is muted.
    pub(crate) fn hides_hashtag(&self, hashtag: &str) -> bool {
        self.hashtags
            .contains(&hashtag.trim().trim_start_matches('#').to_lowercase())
    }

    /// Whether a list drops `event`: its author is muted, a `t` tag is a
    /// muted hashtag, its text holds a muted word, or it is (or answers) a
    /// muted thread. A pure function.
    pub(crate) fn hides(&self, event: &Event) -> bool {
        if self.is_empty() {
            return false;
        }
        if self.hides_key(&event.pubkey) || self.threads.contains(&event.id) {
            return true;
        }
        let tagged = event
            .tags
            .iter()
            .map(|tag| tag.as_slice())
            .any(|tag| match tag {
                [name, value, ..] if name == "t" => self.hides_hashtag(value),
                [name, value, ..] if name == "e" || name == "E" => {
                    EventId::from_hex(value).is_ok_and(|id| self.threads.contains(&id))
                }
                _ => false,
            });
        if tagged {
            return true;
        }
        if self.words.is_empty() {
            return false;
        }
        let text = event.content.to_lowercase();
        self.words.iter().any(|word| text.contains(word.as_str()))
    }

    /// Whether a list drops a repo: its owner is muted, or the announcement
    /// carries a muted topic. `card` is the repo's, when it is loaded.
    pub(crate) fn hides_repo(&self, repo: &RepoAddress, card: Option<&RepoCard>) -> bool {
        self.hides_key(repo.owner())
            || card.is_some_and(|card| {
                topics(card.announcement())
                    .iter()
                    .any(|topic| self.hides_hashtag(topic))
            })
    }
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;

    use super::*;

    fn signed(keys: &Keys, kind: u16, content: &str, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(kind), content)
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn a_mute_list_hides_people_hashtags_words_and_threads() {
        let viewer = Keys::generate();
        let (spammer, friend) = (Keys::generate(), Keys::generate());
        let thread = signed(&friend, 1, "a long argument", &[]);
        let list = signed(
            &viewer,
            MUTE_LIST,
            "encrypted private items are not read",
            &[
                &["p", &spammer.public_key().to_hex()],
                &["p", "not-a-key"],
                &["t", "#Airdrop"],
                &["word", "GIVEAWAY"],
                &["e", &thread.id.to_hex()],
            ],
        );
        let mutes = Mutes::from_list(&list);
        assert_eq!(mutes.counts(), (1, 1));
        assert!(mutes.hides(&signed(&spammer, 1, "hello", &[])));
        assert!(mutes.hides(&signed(&friend, 1, "x", &[&["t", "airdrop"]])));
        assert!(mutes.hides(&signed(&friend, 1, "Huge giveaway today", &[])));
        assert!(mutes.hides(&thread));
        let reply = signed(&friend, 1, "no", &[&["e", &thread.id.to_hex(), "", "root"]]);
        assert!(mutes.hides(&reply));
        assert!(!mutes.hides(&signed(&friend, 1, "shipped", &[&["t", "ship"]])));
        assert!(!Mutes::default().hides(&signed(&spammer, 1, "giveaway", &[])));
    }

    #[test]
    fn a_muted_owner_hides_their_repos() {
        let owner = Keys::generate();
        let list = signed(
            &Keys::generate(),
            MUTE_LIST,
            "",
            &[&["p", &owner.public_key().to_hex()]],
        );
        let repo = RepoAddress::new(owner.public_key(), "r").unwrap();
        let other = RepoAddress::new(Keys::generate().public_key(), "r").unwrap();
        let mutes = Mutes::from_list(&list);
        assert!(mutes.hides_repo(&repo, None));
        assert!(!mutes.hides_repo(&other, None));
    }
}
