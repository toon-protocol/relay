//! Fork lineage of NIP-34 repositories (proposal 7 of
//! `docs/social-for-builders-ux.md`): which repo an announcement says it was
//! forked from, and the network of forks this relay holds.
//!
//! NIP-34 gives a fork two marks (`34.md`, "Repository announcements"):
//!
//! - a `u` tag, `["u", "30617:<pubkey>:<identifier>|<git-url>", <relay>,
//!   <author>]`, saying the repo is a subordinate fork of that announcement;
//! - an `r` tag marked `euc`, the repo's earliest unique commit, which groups
//!   it with copies of the same project hosted elsewhere, `u` tag or not.
//!
//! Both are the forker's own claim, written in the forker's announcement, so
//! a reader takes them as said. Pure functions over events, plus one loader
//! that reads this relay's store and nothing else.

use std::collections::{HashMap, HashSet};

use nostr::event::{Event, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use relay_ui::ReadOnlyStore;

use crate::repo::{ANNOUNCEMENT, RepoAddress, authorized_authors, supersedes};

/// The most announcements one fork network reads: the scan that finds the
/// forks whose `u` tag names a repo (the tag's value carries a git URL after
/// the address, so it cannot be matched by an index).
pub const FORK_SCAN: usize = 2_000;

/// What a fork's `u` tag says it was forked from: the parent's address, and
/// the git URL the forker cloned it from when the tag gives one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForkOf {
    address: RepoAddress,
    git_url: Option<String>,
}

impl ForkOf {
    /// The parent `announcement`'s first `u` tag names, if it names a repo
    /// other than the announcement's own.
    pub fn of(announcement: &Event) -> Option<Self> {
        let own = RepoAddress::of(announcement);
        announcement
            .tags
            .iter()
            .map(|tag| tag.as_slice())
            .filter_map(|tag| match tag {
                [name, value, ..] if name == "u" => Self::parse(value),
                _ => None,
            })
            .find(|parent| Some(&parent.address) != own.as_ref())
    }

    /// A `u` value, `30617:<pubkey>:<identifier>|<git-url>`; the `|` and the
    /// URL may be missing. A pure function.
    pub fn parse(value: &str) -> Option<Self> {
        let (coordinate, url) = match value.split_once('|') {
            Some((coordinate, url)) => (coordinate, Some(url.trim())),
            None => (value, None),
        };
        Some(Self {
            address: RepoAddress::parse(coordinate.trim())?,
            git_url: url.filter(|url| !url.is_empty()).map(str::to_owned),
        })
    }

    /// The parent's address.
    pub fn address(&self) -> &RepoAddress {
        &self.address
    }

    /// The git URL the `u` tag gives for the parent, if any.
    pub fn git_url(&self) -> Option<&str> {
        self.git_url.as_deref()
    }
}

/// The repo's earliest unique commit: its `["r", <commit>, "euc"]` tag, in
/// lower case, if the value is a 40- or 64-digit hex commit id. A pure
/// function.
pub fn earliest_unique_commit(announcement: &Event) -> Option<String> {
    announcement
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find_map(|tag| match tag {
            [name, commit, marker, ..]
                if name == "r"
                    && marker == "euc"
                    && matches!(commit.len(), 40 | 64)
                    && commit.bytes().all(|b| b.is_ascii_hexdigit()) =>
            {
                Some(commit.to_ascii_lowercase())
            }
            _ => None,
        })
}

/// The keys a repo header names: the authorized authors (ADR 0002), and the
/// owner of the repo it was forked from. A pure function.
pub fn named_keys(announcement: &Event) -> Vec<PublicKey> {
    let mut keys = authorized_authors(announcement);
    if let Some(parent) = ForkOf::of(announcement)
        && !keys.contains(parent.address().owner())
    {
        keys.push(*parent.address().owner());
    }
    keys
}

/// The forks this relay holds: each repo's newest announcement by its
/// owner, and which repos name each one in a `u` tag.
#[derive(Debug, Clone, Default)]
pub struct ForkNetwork {
    repos: HashMap<RepoAddress, Event>,
    children: HashMap<RepoAddress, Vec<RepoAddress>>,
}

impl ForkNetwork {
    /// The network `events` describe: the newest kind 30617 per address
    /// (another key's announcement never stands for an owner's), and each
    /// one's forks, oldest first. A pure function.
    pub fn from_announcements(events: &[Event]) -> Self {
        let mut repos: HashMap<RepoAddress, Event> = HashMap::new();
        for event in events
            .iter()
            .filter(|event| event.kind == Kind::from(ANNOUNCEMENT))
        {
            let Some(address) = RepoAddress::of(event) else {
                continue;
            };
            match repos.get(&address) {
                Some(held) if !supersedes(event, held) => {}
                _ => {
                    repos.insert(address, event.clone());
                }
            }
        }
        let mut children: HashMap<RepoAddress, Vec<RepoAddress>> = HashMap::new();
        for (address, announcement) in &repos {
            if let Some(parent) = ForkOf::of(announcement) {
                children
                    .entry(parent.address)
                    .or_default()
                    .push(address.clone());
            }
        }
        for forks in children.values_mut() {
            forks.sort_by(|a, b| {
                let at = |address: &RepoAddress| {
                    repos.get(address).map(|event| (event.created_at, event.id))
                };
                at(a).cmp(&at(b))
            });
        }
        Self { repos, children }
    }

    /// The network around `announcement`: up to [`FORK_SCAN`] announcements,
    /// and every one sharing its earliest unique commit.
    pub async fn load(store: &ReadOnlyStore, announcement: &Event) -> Self {
        let mut events = store
            .query(
                Filter::new()
                    .kind(Kind::from(ANNOUNCEMENT))
                    .limit(FORK_SCAN),
            )
            .await;
        if let Some(commit) = earliest_unique_commit(announcement) {
            events.extend(
                store
                    .query(
                        Filter::new()
                            .kind(Kind::from(ANNOUNCEMENT))
                            .custom_tag(SingleLetterTag::LOWERCASE_R, commit)
                            .limit(FORK_SCAN),
                    )
                    .await,
            );
        }
        events.push(announcement.clone());
        Self::from_announcements(&events)
    }

    /// The announcement held for `address`.
    pub fn announcement(&self, address: &RepoAddress) -> Option<&Event> {
        self.repos.get(address)
    }

    /// What `address`'s announcement says it was forked from.
    pub fn parent(&self, address: &RepoAddress) -> Option<ForkOf> {
        ForkOf::of(self.repos.get(address)?)
    }

    /// The repos whose `u` tag names `address`, oldest first.
    pub fn forks(&self, address: &RepoAddress) -> &[RepoAddress] {
        self.children.get(address).map_or(&[], Vec::as_slice)
    }

    /// `address`'s ancestors, nearest first, at most `max` of them. The walk
    /// stops at a parent this relay does not hold (which is still listed, as
    /// the `u` tag named it) and at a loop.
    pub fn ancestors(&self, address: &RepoAddress, max: usize) -> Vec<ForkOf> {
        let mut found: Vec<ForkOf> = Vec::new();
        let mut seen: HashSet<RepoAddress> = HashSet::from([address.clone()]);
        let mut at = address.clone();
        while found.len() < max {
            let Some(parent) = self.parent(&at) else {
                break;
            };
            if !seen.insert(parent.address.clone()) {
                break;
            }
            at = parent.address.clone();
            found.push(parent);
        }
        found
    }

    /// Every fork below `address`, at most `depth` levels down, each once.
    pub fn descendants(&self, address: &RepoAddress, depth: usize) -> Vec<RepoAddress> {
        let mut found: Vec<RepoAddress> = Vec::new();
        let mut seen: HashSet<RepoAddress> = HashSet::from([address.clone()]);
        let mut level = vec![address.clone()];
        for _ in 0..depth {
            let mut next = Vec::new();
            for parent in &level {
                for fork in self.forks(parent) {
                    if seen.insert(fork.clone()) {
                        found.push(fork.clone());
                        next.push(fork.clone());
                    }
                }
            }
            level = next;
        }
        found
    }

    /// The repos that share `address`'s earliest unique commit but carry no
    /// `u` tag: the same project announced again, or a fork that did not say
    /// so. Oldest first.
    pub fn same_first_commit(&self, address: &RepoAddress) -> Vec<RepoAddress> {
        let Some(commit) = self.repos.get(address).and_then(earliest_unique_commit) else {
            return Vec::new();
        };
        let mut found: Vec<(&RepoAddress, &Event)> = self
            .repos
            .iter()
            .filter(|(other, event)| {
                *other != address
                    && ForkOf::of(event).is_none()
                    && earliest_unique_commit(event).as_deref() == Some(commit.as_str())
            })
            .collect();
        found.sort_by_key(|(_, event)| (event.created_at, event.id));
        found
            .into_iter()
            .map(|(address, _)| address.clone())
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;
    use nostr::types::Timestamp;

    use super::*;

    fn announce(keys: &Keys, d: &str, at: u64, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(ANNOUNCEMENT), "")
            .tags(
                std::iter::once(Tag::parse(["d", d]).unwrap())
                    .chain(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap())),
            )
            .custom_created_at(Timestamp::from(at))
            .finalize(keys)
            .unwrap()
    }

    fn address(keys: &Keys, d: &str) -> RepoAddress {
        RepoAddress::new(keys.public_key(), d).unwrap()
    }

    #[test]
    fn a_u_tag_names_the_parent_with_or_without_its_git_url() {
        let parent = Keys::generate();
        let coordinate = address(&parent, "harmonics").coordinate();
        let with_url = ForkOf::parse(&format!("{coordinate}|https://git.test/h.git")).unwrap();
        assert_eq!(with_url.address(), &address(&parent, "harmonics"));
        assert_eq!(with_url.git_url(), Some("https://git.test/h.git"));
        let bare = ForkOf::parse(&coordinate).unwrap();
        assert_eq!(bare.git_url(), None);
        assert_eq!(ForkOf::parse("https://git.test/h.git"), None);
        assert_eq!(ForkOf::parse("30023:00:x|https://x"), None);
    }

    #[test]
    fn a_repo_is_not_its_own_fork() {
        let owner = Keys::generate();
        let own = address(&owner, "r").coordinate();
        let selfish = announce(&owner, "r", 1, &[&["u", &own]]);
        assert_eq!(ForkOf::of(&selfish), None);
    }

    #[test]
    fn the_earliest_unique_commit_is_the_r_tag_marked_euc() {
        let owner = Keys::generate();
        let commit = "A".repeat(40);
        let repo = announce(
            &owner,
            "r",
            1,
            &[&["r", &"b".repeat(40)], &["r", &commit, "euc"]],
        );
        assert_eq!(earliest_unique_commit(&repo), Some("a".repeat(40)));
        let bad = announce(&owner, "r", 1, &[&["r", "not-a-commit", "euc"]]);
        assert_eq!(earliest_unique_commit(&bad), None);
    }

    #[test]
    fn the_network_links_forks_to_their_parents_and_groups_by_first_commit() {
        let (noaa, maya, ruth, dan, kofi) = (
            Keys::generate(),
            Keys::generate(),
            Keys::generate(),
            Keys::generate(),
            Keys::generate(),
        );
        let origin = address(&noaa, "harmonics");
        let tidepool = address(&maya, "tidepool");
        let euc = "c".repeat(40);
        let events = vec![
            announce(&noaa, "harmonics", 1, &[]),
            // An older announcement of tidepool, replaced by the newer one.
            announce(&maya, "tidepool", 2, &[]),
            announce(
                &maya,
                "tidepool",
                3,
                &[
                    &["u", &format!("{}|https://git.test/h", origin.coordinate())],
                    &["r", &euc, "euc"],
                ],
            ),
            announce(&kofi, "harmonics-rs", 4, &[&["u", &origin.coordinate()]]),
            announce(
                &ruth,
                "tidepool",
                5,
                &[&["u", &tidepool.coordinate()], &["r", &euc, "euc"]],
            ),
            // Same first commit, no u tag.
            announce(&dan, "tidepool-nz", 6, &[&["r", &euc, "euc"]]),
        ];
        let network = ForkNetwork::from_announcements(&events);
        assert_eq!(
            network.forks(&origin),
            &[tidepool.clone(), address(&kofi, "harmonics-rs")]
        );
        assert_eq!(network.forks(&tidepool), &[address(&ruth, "tidepool")]);
        let ancestors = network.ancestors(&address(&ruth, "tidepool"), 8);
        assert_eq!(
            ancestors
                .iter()
                .map(|parent| parent.address().clone())
                .collect::<Vec<_>>(),
            vec![tidepool.clone(), origin.clone()]
        );
        assert_eq!(network.descendants(&origin, 1).len(), 2);
        assert_eq!(network.descendants(&origin, 2).len(), 3);
        // Ruth's fork says so with a u tag, so only Dan's is grouped by commit.
        assert_eq!(
            network.same_first_commit(&tidepool),
            vec![address(&dan, "tidepool-nz")]
        );
    }

    #[test]
    fn a_loop_of_u_tags_ends_the_walk() {
        let (a, b) = (Keys::generate(), Keys::generate());
        let events = vec![
            announce(&a, "x", 1, &[&["u", &address(&b, "y").coordinate()]]),
            announce(&b, "y", 2, &[&["u", &address(&a, "x").coordinate()]]),
        ];
        let network = ForkNetwork::from_announcements(&events);
        assert_eq!(network.ancestors(&address(&a, "x"), 8).len(), 1);
        assert_eq!(network.descendants(&address(&a, "x"), 8).len(), 1);
    }

    #[test]
    fn a_header_names_the_parent_owner_too() {
        let (owner, parent) = (Keys::generate(), Keys::generate());
        let fork = announce(
            &owner,
            "r",
            1,
            &[&["u", &address(&parent, "p").coordinate()]],
        );
        assert_eq!(
            named_keys(&fork),
            vec![owner.public_key(), parent.public_key()]
        );
    }
}
