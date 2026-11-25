//! A reader's view: what one reader may see of the store.
//!
//! A reader is anonymous or holds proven keys (NIP-42). What it may see is
//! the NIP-42 policy's and the NIP-29 groups' rules together. The WebSocket
//! session asks them here, in two forms that agree: the conditions a store
//! query carries, so `limit` counts only what the reader may see, and a check
//! of one event, for the live events that reach a subscription without a
//! query.

use std::collections::HashSet;

use nostr::event::Event;
use nostr::key::PublicKey;

use crate::auth::AuthPolicy;
use crate::groups::GroupView;
use crate::store::{Hidden, Query};

/// The relay's rules for readers: NIP-42's policy and NIP-29's groups, each
/// while it is on.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Rules<'a> {
    pub(crate) policy: Option<&'a AuthPolicy>,
    pub(crate) groups: Option<&'a GroupView>,
}

impl Rules<'_> {
    /// The conditions of a query for a reader that has proven `keys`.
    pub(crate) fn conditions(&self, keys: &HashSet<PublicKey>) -> Conditions {
        Conditions {
            wrap_recipients: self
                .policy
                .filter(|policy| policy.is_recipient_only())
                .map(|_| keys.iter().map(PublicKey::to_hex).collect()),
            hidden: self
                .groups
                .map_or_else(Vec::new, |groups| groups.hidden_from(keys)),
        }
    }

    /// Whether a reader that has proven `keys` may see `event`.
    pub(crate) fn may_see(&self, event: &Event, keys: &HashSet<PublicKey>) -> bool {
        self.policy
            .is_none_or(|policy| policy.may_read(event, keys))
            && self
                .groups
                .is_none_or(|groups| groups.may_read(event, keys))
    }
}

/// What a store query carries so that it returns only what a reader may see.
#[derive(Debug, Clone, Default)]
pub(crate) struct Conditions {
    wrap_recipients: Option<Vec<String>>,
    hidden: Vec<Hidden>,
}

impl Conditions {
    /// `query`, with these conditions added.
    pub(crate) fn apply(&self, mut query: Query) -> Query {
        query.wrap_recipients.clone_from(&self.wrap_recipients);
        query.hidden.extend(self.hidden.iter().cloned());
        query
    }
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Kind, Tag};
    use nostr::filter::Filter;
    use nostr::key::Keys;
    use nostr::types::Timestamp;
    use tempfile::tempdir;

    use super::*;
    use crate::{Store, VerifiedEvent};

    const CREATE_GROUP: u16 = 9007;

    fn signed(keys: &Keys, kind: u16, at: u64, tags: &[&[&str]]) -> Event {
        let tags = tags
            .iter()
            .map(|tag| Tag::parse(tag.iter().copied()).expect("a non-empty tag parses"));
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags)
            .custom_created_at(Timestamp::from(at))
            .finalize(keys)
            .expect("a generated key signs an event")
    }

    /// A relay with a private group, a closed one and an open one, holding
    /// notes, chat, group metadata and gift wraps of all of them, and the
    /// people who read it.
    struct World {
        store: Store,
        groups: GroupView,
        member: PublicKey,
        wrapped: PublicKey,
        stranger: PublicKey,
        _dir: tempfile::TempDir,
    }

    async fn world() -> World {
        let (owner, wrapped, stranger) = (Keys::generate(), Keys::generate(), Keys::generate());
        let author = Keys::generate();
        let groups = GroupView::built_by(&[
            signed(&owner, CREATE_GROUP, 1, &[&["h", "secret"], &["private"]]),
            signed(&owner, CREATE_GROUP, 2, &[&["h", "shut"], &["closed"]]),
            signed(&owner, CREATE_GROUP, 3, &[&["h", "open"]]),
        ]);
        let wrapped_hex = wrapped.public_key().to_hex();
        let owner_hex = owner.public_key().to_hex();
        let mut events = Vec::new();
        let mut at = 100;
        for group in [None, Some("secret"), Some("shut"), Some("open")] {
            for kind in [1u16, 9] {
                for _ in 0..3 {
                    at += 1;
                    events.push(match group {
                        Some(group) => signed(&author, kind, at, &[&["h", group]]),
                        None => signed(&author, kind, at, &[]),
                    });
                }
            }
        }
        for group in ["secret", "shut", "open", "unknown"] {
            at += 1;
            events.push(signed(&owner, 39000, at, &[&["d", group]]));
        }
        for recipient in [&wrapped_hex, &owner_hex] {
            for _ in 0..3 {
                at += 1;
                events.push(signed(&author, 1059, at, &[&["p", recipient]]));
            }
        }
        let dir = tempdir().expect("a temp dir");
        let store = Store::open(&dir.path().join("relay.db")).expect("a store opens");
        for event in &events {
            let event = VerifiedEvent::verify(event.clone()).expect("the event verifies");
            store.save(&event).await.expect("the event is saved");
        }
        World {
            store,
            groups,
            member: owner.public_key(),
            wrapped: wrapped.public_key(),
            stranger: stranger.public_key(),
            _dir: dir,
        }
    }

    /// A small deterministic source of choices, so a failing case is the
    /// same case on every run.
    struct Choices(u64);

    impl Choices {
        /// The next of `n` choices (splitmix64).
        fn below(&mut self, n: usize) -> usize {
            self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
            let mut z = self.0;
            z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
            let n = u64::try_from(n).expect("a small count");
            usize::try_from((z ^ (z >> 31)) % n).expect("below a usize")
        }

        fn pick<'a, T>(&mut self, of: &'a [T]) -> &'a T {
            &of[self.below(of.len())]
        }

        fn some<T: Clone>(&mut self, of: &[T]) -> Vec<T> {
            of.iter().filter(|_| self.below(2) == 0).cloned().collect()
        }
    }

    const GROUPS: [&str; 4] = ["secret", "shut", "open", "unknown"];
    const KINDS: [u16; 6] = [1, 9, 1059, 39000, 39001, 30023];

    #[tokio::test]
    async fn for_generated_readers_filters_and_events_a_limit_counts_only_what_the_check_shows() {
        let world = world().await;
        let people = [world.member, world.wrapped, world.stranger];
        let author = Keys::generate();
        let policy = AuthPolicy::default().recipient_only(true);
        let mut choices = Choices(369);
        for _ in 0..48 {
            let dir = tempdir().expect("a temp dir");
            let store = Store::open(&dir.path().join("relay.db")).expect("a store opens");
            for at in 0..choices.below(40) {
                let kind = *choices.pick(&KINDS);
                let mut tags: Vec<Vec<String>> = Vec::new();
                for _ in 0..choices.below(4) {
                    let name = *choices.pick(&["h", "d", "p"]);
                    let value = match name {
                        "p" => choices.pick(&people).to_hex(),
                        _ => (*choices.pick(&GROUPS)).to_string(),
                    };
                    tags.push(vec![name.to_string(), value]);
                }
                let tags: Vec<Vec<&str>> = tags
                    .iter()
                    .map(|tag| tag.iter().map(String::as_str).collect())
                    .collect();
                let tags: Vec<&[&str]> = tags.iter().map(Vec::as_slice).collect();
                let event = signed(
                    &author,
                    kind,
                    1000 + u64::try_from(at).expect("small"),
                    &tags,
                );
                let event = VerifiedEvent::verify(event).expect("the event verifies");
                store.save(&event).await.expect("the event is saved");
            }
            let stored = store.query(Filter::new()).await.expect("it runs");

            let rules = Rules {
                policy: *choices.pick(&[None, Some(&policy)]),
                groups: *choices.pick(&[None, Some(&world.groups)]),
            };
            let keys: HashSet<PublicKey> = choices.some(&people).into_iter().collect();
            let mut filter = Filter::new();
            let kinds = choices.some(&KINDS);
            if !kinds.is_empty() {
                filter = filter.kinds(kinds.into_iter().map(Kind::from));
            }
            if choices.below(3) == 0 {
                let group = *choices.pick(&GROUPS);
                filter = filter.custom_tag(
                    nostr::filter::SingleLetterTag::from_char('h').expect("h is a tag letter"),
                    group,
                );
            }
            let visible: Vec<&Event> = stored
                .iter()
                .filter(|event| filter.match_event(event, nostr::filter::MatchEventOptions::new()))
                .filter(|event| rules.may_see(event, &keys))
                .collect();

            let conditions = rules.conditions(&keys);
            let unlimited = conditions.apply(Query::from(filter.clone()));
            let found = store.query(unlimited).await.expect("it runs");
            assert_eq!(
                found.iter().collect::<Vec<_>>(),
                visible,
                "the conditions and the check disagree for {filter:?}"
            );
            let limit = choices.below(12);
            let query = conditions.apply(Query::from(filter.clone().limit(limit)));
            let found = store.query(query).await.expect("it runs");
            assert_eq!(found.len(), limit.min(visible.len()));
            assert!(found.iter().all(|event| rules.may_see(event, &keys)));
        }
    }

    #[tokio::test]
    async fn an_anonymous_reader_sees_no_wrap_and_no_private_group_and_keys_see_their_own() {
        let world = world().await;
        let policy = AuthPolicy::default().recipient_only(true);
        let rules = Rules {
            policy: Some(&policy),
            groups: Some(&world.groups),
        };
        let query = |keys: &HashSet<PublicKey>| {
            let query = Query::from(Filter::new());
            let query = rules.conditions(keys).apply(query);
            world.store.query(query)
        };
        let anonymous = query(&HashSet::new()).await.expect("it runs");
        assert!(anonymous.iter().all(|event| event.kind != Kind::GiftWrap));
        assert!(anonymous.iter().all(|event| {
            !event.tags.iter().any(|tag| {
                tag.kind() == "h" && ["secret", "shut"].contains(&tag.content().unwrap_or(""))
            })
        }));

        let keys = HashSet::from([world.member]);
        let own = query(&keys).await.expect("it runs");
        let wraps: Vec<_> = own.iter().filter(|e| e.kind == Kind::GiftWrap).collect();
        assert_eq!(wraps.len(), 3);
        assert!(
            wraps
                .iter()
                .all(|wrap| wrap.tags.public_keys().any(|key| keys.contains(&key)))
        );
        assert_eq!(
            own.iter()
                .filter(|e| e
                    .tags
                    .iter()
                    .any(|t| t.kind() == "h" && t.content() == Some("secret")))
                .count(),
            6
        );
    }
}
