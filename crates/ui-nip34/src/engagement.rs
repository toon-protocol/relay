//! Repo engagement: who starred, followed or forked a repo, read from this
//! relay's store, for one repo or many.
//!
//! One rule per act, and one query plan per act that serves one repo and many
//! alike, so a repo's page and a list that shows it cannot disagree:
//!
//! - a **star** is a kind 7 `+` that names the repo in an `a` tag and `30617`
//!   in a `k` tag ([`is_star`]);
//! - a **follow** is a kind 10018 that names the repo in an `a` tag. The kind
//!   is replaceable, so the store holds each key's newest list and a list that
//!   dropped the repo no longer counts;
//! - a **fork** is a kind 30617 whose `u` tag, read by [`ForkOf::parse`],
//!   names the repo, with or without `|<git url>` after it, and which is not
//!   the repo itself ([`forked_from`]).
//!
//! A star or a follow counts a key once, however often it wrote; a fork counts
//! a repo once. A NIP-09 deletion removes an event from the store, so an
//! un-star subtracts. A caller asks only for the acts it shows ([`Act`]), so
//! it costs one query per act, and no more than the page it replaced.

use std::collections::{HashMap, HashSet};

use nostr::event::{Event, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use nostr::types::Timestamp;
use relay_ui::reading::newest;
use relay_ui::{ReadOnlyStore, StoreError};

use crate::lineage::ForkOf;
use crate::repo::{ANNOUNCEMENT, RepoAddress};

/// NIP-25 reaction: a star when it is `+` on a repo.
pub const REACTION: u16 = 7;
/// NIP-51 git repositories: the repos a key follows.
pub const FOLLOWED_REPOS: u16 = 10_018;

/// The most events one act's query reads, however many repos it is asked
/// about. The one limit of the plan: a repo's answer is the same whether it is
/// asked alone or among others.
const LIMIT: usize = 20_000;

/// One thing a person can do to a repo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Act {
    /// Star it.
    Star,
    /// Follow it.
    Follow,
    /// Fork it.
    Fork,
}

/// A repo that forks another: its address and when its announcement was
/// written. Its owner is the key that forked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fork {
    /// The fork's own address.
    pub repo: RepoAddress,
    /// When the fork's announcement was written.
    pub at: Timestamp,
}

/// What people did to one repo. An act that was not asked for is empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Engagement {
    stars: Vec<(PublicKey, Timestamp)>,
    followers: Vec<(PublicKey, Timestamp)>,
    forks: Vec<Fork>,
}

impl Engagement {
    /// The people who starred it, newest first, each once.
    pub fn stars(&self) -> &[(PublicKey, Timestamp)] {
        &self.stars
    }

    /// The people whose newest followed-repos list names it, newest first.
    pub fn followers(&self) -> &[(PublicKey, Timestamp)] {
        &self.followers
    }

    /// The repos that fork it, newest first, each once.
    pub fn forks(&self) -> &[Fork] {
        &self.forks
    }

    /// How many people starred it.
    pub fn star_count(&self) -> usize {
        self.stars.len()
    }

    /// How many people starred it other than `besides` (a repo's makers, who
    /// do not count for their own repo).
    pub fn star_count_besides(&self, besides: &[PublicKey]) -> usize {
        self.stars
            .iter()
            .filter(|(key, _)| !besides.contains(key))
            .count()
    }

    /// How many people follow it.
    pub fn follower_count(&self) -> usize {
        self.followers.len()
    }

    /// How many repos fork it.
    pub fn fork_count(&self) -> usize {
        self.forks.len()
    }
}

/// What people did to each of several repos.
#[derive(Debug, Clone, Default)]
pub struct Engagements {
    repos: HashMap<RepoAddress, Engagement>,
}

impl Engagements {
    /// What people did to `repo`; nothing for a repo that was not asked about.
    pub fn of(&self, repo: &RepoAddress) -> &Engagement {
        static NONE: Engagement = Engagement {
            stars: Vec::new(),
            followers: Vec::new(),
            forks: Vec::new(),
        };
        self.repos.get(repo).unwrap_or(&NONE)
    }

    /// The events of each act, read into one [`Engagement`] per repo. A pure
    /// function: [`load`] is the queries and this.
    fn read(repos: &[RepoAddress], reactions: &[Event], lists: &[Event], forks: &[Event]) -> Self {
        let parents: Vec<(RepoAddress, &Event)> = forks
            .iter()
            .filter_map(|event| Some((forked_from(event)?, event)))
            .collect();
        let repos = repos
            .iter()
            .map(|repo| {
                let coordinate = repo.coordinate();
                let forked: Vec<&Event> = parents
                    .iter()
                    .filter(|(parent, _)| parent == repo)
                    .map(|(_, event)| *event)
                    .collect();
                let engagement = Engagement {
                    stars: people(reactions, |event| is_star(event, &coordinate)),
                    followers: people(lists, |event| follows(event, &coordinate)),
                    forks: fork_repos(&forked),
                };
                (repo.clone(), engagement)
            })
            .collect();
        Self { repos }
    }
}

/// What people did to each of `repos`, as far as `acts` asks: one query per
/// act, whatever the number of repos.
pub async fn load(
    store: &ReadOnlyStore,
    repos: &[RepoAddress],
    acts: &[Act],
) -> Result<Engagements, StoreError> {
    if repos.is_empty() || acts.is_empty() {
        return Ok(Engagements::default());
    }
    let coordinates: Vec<String> = repos.iter().map(RepoAddress::coordinate).collect();
    let coordinates = &coordinates;
    let ask = |act: Act| async move {
        if acts.contains(&act) {
            store.query(plan(act, coordinates)).await
        } else {
            Ok(Vec::new())
        }
    };
    let reactions = ask(Act::Star).await?;
    let lists = ask(Act::Follow).await?;
    let forks = ask(Act::Fork).await?;
    Ok(Engagements::read(repos, &reactions, &lists, &forks))
}

/// What people did to `repo`, as far as `acts` asks.
pub async fn load_one(
    store: &ReadOnlyStore,
    repo: &RepoAddress,
    acts: &[Act],
) -> Result<Engagement, StoreError> {
    Ok(load(store, std::slice::from_ref(repo), acts)
        .await?
        .repos
        .remove(repo)
        .unwrap_or_default())
}

/// The repos `key` starred, newest first, each once.
pub async fn starred_by(
    store: &ReadOnlyStore,
    key: &PublicKey,
) -> Result<Vec<RepoAddress>, StoreError> {
    let reactions = store
        .query(
            Filter::new()
                .kind(Kind::from(REACTION))
                .author(*key)
                .custom_tag(SingleLetterTag::LOWERCASE_K, ANNOUNCEMENT.to_string())
                .limit(LIMIT),
        )
        .await?;
    Ok(starred(key, &reactions))
}

/// The query for `act` on repos at `coordinates`: the one place a limit is
/// set. A fork is not found by its `u` tag: the tag may carry `|<git url>`
/// after the coordinate, which an exact `#u` match misses (#354), so the plan
/// reads announcements and [`forked_from`] reads each.
fn plan(act: Act, coordinates: &[String]) -> Filter {
    let by_coordinate = |kind: u16| {
        Filter::new()
            .kind(Kind::from(kind))
            .custom_tags(SingleLetterTag::LOWERCASE_A, coordinates.to_vec())
    };
    match act {
        Act::Star => by_coordinate(REACTION),
        Act::Follow => by_coordinate(FOLLOWED_REPOS),
        Act::Fork => Filter::new().kind(Kind::from(ANNOUNCEMENT)),
    }
    .limit(LIMIT)
}

/// Whether `event` names `value` in a tag called `name` (relay hints and
/// markers after the value are ignored).
fn names_value(event: &Event, name: &str, value: &str) -> bool {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .any(|tag| matches!(tag, [n, v, ..] if n == name && v == value))
}

/// Whether `event` stars the repo at `coordinate`: a kind 7 whose content is
/// `+`, with the repo in an `a` tag and `30617` in a `k` tag. A pure function.
pub fn is_star(event: &Event, coordinate: &str) -> bool {
    event.kind == Kind::from(REACTION)
        && event.content.trim() == "+"
        && names_value(event, "a", coordinate)
        && names_value(event, "k", &ANNOUNCEMENT.to_string())
}

/// Whether `event` is a followed-repos list (kind 10018) that names the repo
/// at `coordinate`. A pure function.
pub fn follows(event: &Event, coordinate: &str) -> bool {
    event.kind == Kind::from(FOLLOWED_REPOS) && names_value(event, "a", coordinate)
}

/// The repo `announcement` forks: its `u` tag, when that names another repo's
/// address, with or without `|<git url>` after it (a `u` that is only a git
/// URL names none). An announcement is never a fork of itself. A pure
/// function.
pub fn forked_from(announcement: &Event) -> Option<RepoAddress> {
    if announcement.kind != Kind::from(ANNOUNCEMENT) {
        return None;
    }
    ForkOf::of(announcement).map(|parent| parent.address().clone())
}

/// Each author among `events` that `admits` once, with the time of their
/// newest such event, newest first (a tie to the lower key). A pure function.
pub fn people(events: &[Event], admits: impl Fn(&Event) -> bool) -> Vec<(PublicKey, Timestamp)> {
    let mut found: Vec<(PublicKey, Timestamp)> = Vec::new();
    for event in events.iter().filter(|event| admits(event)) {
        match found.iter_mut().find(|(key, _)| *key == event.pubkey) {
            Some((_, at)) => *at = (*at).max(event.created_at),
            None => found.push((event.pubkey, event.created_at)),
        }
    }
    found.sort_by(|(a, at_a), (b, at_b)| at_b.cmp(at_a).then_with(|| a.to_hex().cmp(&b.to_hex())));
    found
}

/// The forks among `announcements` (all of one parent): each repo once, by its
/// newest announcement, newest first (a tie to the lower address).
fn fork_repos(announcements: &[&Event]) -> Vec<Fork> {
    let repos: HashSet<RepoAddress> = announcements
        .iter()
        .filter_map(|event| RepoAddress::of(event))
        .collect();
    let mut forks: Vec<Fork> = repos
        .into_iter()
        .filter_map(|repo| {
            let held = newest(
                announcements
                    .iter()
                    .copied()
                    .filter(|event| RepoAddress::of(event).as_ref() == Some(&repo)),
            )?;
            Some(Fork {
                at: held.created_at,
                repo,
            })
        })
        .collect();
    forks.sort_by(|a, b| {
        b.at.cmp(&a.at)
            .then_with(|| a.repo.coordinate().cmp(&b.repo.coordinate()))
    });
    forks
}

/// The repos `key` starred among `reactions`, newest first, each once. A pure
/// function.
fn starred(key: &PublicKey, reactions: &[Event]) -> Vec<RepoAddress> {
    let mut reactions: Vec<&Event> = reactions
        .iter()
        .filter(|event| event.pubkey == *key)
        .collect();
    reactions.sort_by(|a, b| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
    let mut found: Vec<RepoAddress> = Vec::new();
    for event in reactions {
        for repo in RepoAddress::tagged(event, "a") {
            if is_star(event, &repo.coordinate()) && !found.contains(&repo) {
                found.push(repo);
            }
        }
    }
    found
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::filter::MatchEventOptions;
    use nostr::key::Keys;
    use relay_ui::EventSource;
    use relay_ui::reading::{d_tag, supersedes};

    use super::*;

    /// An in-memory store that keeps only the newest event of a replaceable or
    /// addressable slot, as the relay's does (ADR 0002), and counts queries.
    #[derive(Clone, Default)]
    struct Memory {
        events: Arc<Vec<Event>>,
        queries: Arc<AtomicUsize>,
    }

    impl Memory {
        fn holding(saved: Vec<Event>) -> Self {
            let mut kept: Vec<Event> = Vec::new();
            for event in saved {
                let kind = event.kind.as_u16();
                let slot = (10_000..20_000).contains(&kind) || (30_000..40_000).contains(&kind);
                let held = kept.iter().position(|held| {
                    slot && held.kind == event.kind
                        && held.pubkey == event.pubkey
                        && d_tag(held) == d_tag(&event)
                });
                match held {
                    Some(at) if supersedes(&event, &kept[at]) => kept[at] = event,
                    Some(_) => {}
                    None => kept.push(event),
                }
            }
            Self {
                events: Arc::new(kept),
                queries: Arc::default(),
            }
        }

        fn store(&self) -> ReadOnlyStore {
            ReadOnlyStore::new(self.clone())
        }

        fn queries(&self) -> usize {
            self.queries.load(Ordering::SeqCst)
        }
    }

    impl EventSource for Memory {
        fn query(
            &self,
            filter: Filter,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Event>, StoreError>> + Send + '_>> {
            self.queries.fetch_add(1, Ordering::SeqCst);
            let mut found: Vec<Event> = self
                .events
                .iter()
                .filter(|event| filter.match_event(event, MatchEventOptions::new()))
                .cloned()
                .collect();
            found.sort_by(|a, b| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
            found.truncate(filter.limit.unwrap_or(usize::MAX));
            Box::pin(async move { Ok(found) })
        }
    }

    fn signed(keys: &Keys, kind: u16, at: u64, content: &str, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(kind), content)
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .custom_created_at(Timestamp::from(at))
            .finalize(keys)
            .unwrap()
    }

    fn star(keys: &Keys, at: u64, coordinate: &str) -> Event {
        signed(keys, 7, at, "+", &[&["a", coordinate], &["k", "30617"]])
    }

    fn announce(keys: &Keys, d: &str, at: u64, u: Option<&str>) -> Event {
        match u {
            Some(u) => signed(keys, 30_617, at, "", &[&["d", d], &["u", u]]),
            None => signed(keys, 30_617, at, "", &[&["d", d]]),
        }
    }

    fn repo(keys: &Keys, d: &str) -> RepoAddress {
        RepoAddress::new(keys.public_key(), d).unwrap()
    }

    const EVERYTHING: [Act; 3] = [Act::Star, Act::Follow, Act::Fork];

    #[tokio::test]
    async fn a_fork_in_the_documented_form_counts_toward_its_parent() {
        let (owner, forker, other) = (Keys::generate(), Keys::generate(), Keys::generate());
        let parent = repo(&owner, "r");
        let c = parent.coordinate();
        let memory = Memory::holding(vec![
            announce(&owner, "r", 1, None),
            announce(
                &forker,
                "r",
                2,
                Some(&format!("{c}|https://example.com/r.git")),
            ),
            announce(&other, "plain", 3, Some(&c)),
            announce(&other, "url", 4, Some("https://example.com/r.git")),
        ]);
        let forks = load_one(&memory.store(), &parent, &[Act::Fork])
            .await
            .unwrap();
        let found: Vec<&RepoAddress> = forks.forks().iter().map(|fork| &fork.repo).collect();
        assert_eq!(found, [&repo(&other, "plain"), &repo(&forker, "r")]);
        assert_eq!(forks.fork_count(), 2);
    }

    #[tokio::test]
    async fn a_repos_own_announcement_is_never_its_own_fork() {
        let owner = Keys::generate();
        let own = repo(&owner, "r");
        let c = own.coordinate();
        let memory = Memory::holding(vec![announce(
            &owner,
            "r",
            1,
            Some(&format!("{c}|https://example.com/r.git")),
        )]);
        let found = load_one(&memory.store(), &own, &[Act::Fork]).await.unwrap();
        assert_eq!(found.fork_count(), 0);
    }

    #[tokio::test]
    async fn a_star_needs_a_plus_the_repo_and_its_kind_and_counts_a_key_once() {
        let owner = Keys::generate();
        let (fan, other_fan) = (Keys::generate(), Keys::generate());
        let target = repo(&owner, "r");
        let a = target.coordinate();
        let elsewhere = repo(&owner, "other").coordinate();
        let memory = Memory::holding(vec![
            star(&fan, 10, &a),
            star(&fan, 30, &a),
            signed(&other_fan, 7, 20, "-", &[&["a", &a], &["k", "30617"]]),
            signed(&other_fan, 7, 21, "+", &[&["a", &a]]),
            signed(&other_fan, 7, 22, "+", &[&["e", &a], &["k", "30617"]]),
            star(&other_fan, 23, &elsewhere),
        ]);
        let found = load_one(&memory.store(), &target, &[Act::Star])
            .await
            .unwrap();
        assert_eq!(found.stars(), [(fan.public_key(), Timestamp::from(30))]);
    }

    #[tokio::test]
    async fn stars_from_others_leave_the_makers_out() {
        let (owner, fan) = (Keys::generate(), Keys::generate());
        let target = repo(&owner, "r");
        let a = target.coordinate();
        let memory = Memory::holding(vec![star(&owner, 1, &a), star(&fan, 2, &a)]);
        let found = load_one(&memory.store(), &target, &[Act::Star])
            .await
            .unwrap();
        assert_eq!(found.star_count(), 2);
        assert_eq!(found.star_count_besides(&[owner.public_key()]), 1);
    }

    #[tokio::test]
    async fn the_answer_for_one_repo_is_its_entry_in_the_answer_for_many() {
        let (owner, fan, forker) = (Keys::generate(), Keys::generate(), Keys::generate());
        let (a, b, c) = (repo(&owner, "a"), repo(&owner, "b"), repo(&owner, "c"));
        let memory = Memory::holding(vec![
            star(&fan, 1, &a.coordinate()),
            star(&forker, 2, &a.coordinate()),
            star(&fan, 3, &b.coordinate()),
            signed(
                &fan,
                10_018,
                4,
                "",
                &[&["a", &a.coordinate()], &["a", &c.coordinate()]],
            ),
            announce(&forker, "a", 5, Some(&format!("{}|x", a.coordinate()))),
            announce(&forker, "c", 6, Some(&c.coordinate())),
        ]);
        let store = memory.store();
        let many = load(&store, &[a.clone(), b.clone(), c.clone()], &EVERYTHING)
            .await
            .unwrap();
        for repo in [&a, &b, &c] {
            let one = load_one(&store, repo, &EVERYTHING).await.unwrap();
            assert_eq!(&one, many.of(repo));
        }
        assert_eq!(many.of(&a).star_count(), 2);
        assert_eq!(many.of(&a).fork_count(), 1);
        assert_eq!(many.of(&c).follower_count(), 1);
        assert_eq!(many.of(&b).fork_count(), 0);
    }

    #[tokio::test]
    async fn a_follower_whose_newer_list_drops_the_repo_is_no_longer_one() {
        let owner = Keys::generate();
        let (stays, leaves) = (Keys::generate(), Keys::generate());
        let target = repo(&owner, "r");
        let a = target.coordinate();
        let memory = Memory::holding(vec![
            signed(&stays, 10_018, 1, "", &[&["a", &a]]),
            signed(&leaves, 10_018, 1, "", &[&["a", &a]]),
            signed(
                &leaves,
                10_018,
                2,
                "",
                &[&["a", &repo(&owner, "x").coordinate()]],
            ),
        ]);
        let found = load_one(&memory.store(), &target, &[Act::Follow])
            .await
            .unwrap();
        assert_eq!(
            found.followers(),
            [(stays.public_key(), Timestamp::from(1))]
        );
    }

    #[tokio::test]
    async fn each_act_costs_one_query_for_one_repo_or_many() {
        let owner = Keys::generate();
        let repos: Vec<RepoAddress> = (0..5).map(|n| repo(&owner, &format!("r{n}"))).collect();
        let memory = Memory::holding(vec![]);
        load(&memory.store(), &repos, &EVERYTHING).await.unwrap();
        assert_eq!(memory.queries(), 3);
        load_one(&memory.store(), &repos[0], &[Act::Star, Act::Fork])
            .await
            .unwrap();
        assert_eq!(memory.queries(), 5);
    }

    #[tokio::test]
    async fn the_repos_a_key_starred_come_newest_first_each_once() {
        let (me, owner) = (Keys::generate(), Keys::generate());
        let a = repo(&owner, "r").coordinate();
        let b = repo(&owner, "s").coordinate();
        let memory = Memory::holding(vec![
            star(&me, 1, &a),
            star(&me, 2, &a),
            signed(&me, 7, 3, "-", &[&["a", &b], &["k", "30617"]]),
            star(&me, 4, &b),
            star(&owner, 5, &a),
        ]);
        let found = starred_by(&memory.store(), &me.public_key()).await.unwrap();
        assert_eq!(
            found,
            [
                RepoAddress::parse(&b).unwrap(),
                RepoAddress::parse(&a).unwrap()
            ]
        );
    }
}
