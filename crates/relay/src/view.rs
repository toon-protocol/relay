//! A reader's view: what one reader may see of the store.
//!
//! A reader is anonymous or holds proven keys (NIP-42). What it may see is
//! the NIP-42 policy's and the NIP-29 groups' rules together. They are asked
//! here and nowhere else, in two forms that agree: the conditions a store
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
        events: Vec<Event>,
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
            events,
            groups,
            member: owner.public_key(),
            wrapped: wrapped.public_key(),
            stranger: stranger.public_key(),
            _dir: dir,
        }
    }

    fn filters() -> Vec<Filter> {
        vec![
            Filter::new(),
            Filter::new().kind(Kind::from(1)),
            Filter::new().kinds([Kind::from(1), Kind::from(9)]),
            Filter::new().kind(Kind::from(1059)),
            Filter::new().kind(Kind::from(39000)),
        ]
    }

    #[tokio::test]
    async fn a_limit_counts_only_what_the_reader_may_see_and_the_check_agrees() {
        let world = world().await;
        let readers = [
            HashSet::new(),
            HashSet::from([world.member]),
            HashSet::from([world.wrapped]),
            HashSet::from([world.stranger]),
            HashSet::from([world.member, world.wrapped]),
        ];
        let policy = AuthPolicy::default().recipient_only(true);
        for policy in [None, Some(&policy)] {
            let rules = Rules {
                policy,
                groups: Some(&world.groups),
            };
            for keys in &readers {
                let conditions = rules.conditions(keys);
                for filter in filters() {
                    let mut visible: Vec<&Event> = world
                        .events
                        .iter()
                        .filter(|event| {
                            filter.match_event(event, nostr::filter::MatchEventOptions::new())
                        })
                        .filter(|event| rules.may_see(event, keys))
                        .collect();
                    visible.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
                    let unlimited = conditions.apply(Query::from(filter.clone()));
                    let found = world.store.query(unlimited).await.expect("it runs");
                    assert_eq!(
                        found.iter().collect::<Vec<_>>(),
                        visible,
                        "the conditions and the check disagree for {filter:?}"
                    );
                    for limit in [1, 2, 5, 100] {
                        let query = conditions.apply(Query::from(filter.clone().limit(limit)));
                        let found = world.store.query(query).await.expect("it runs");
                        assert_eq!(found.len(), limit.min(visible.len()));
                        assert!(found.iter().all(|event| rules.may_see(event, keys)));
                    }
                }
            }
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
