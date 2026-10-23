//! The repo card: what a mention of a repository unfurls into wherever an app
//! shows one (proposal 1 of `docs/social-for-builders-ux.md`), and the
//! social signals it counts.
//!
//! A card is read from this relay's store alone: the owner's announcement,
//! the honoured repo state (ADR 0002), stars, followers and open threads. It
//! counts people, not events: a star is a kind 7 `+` that names the repo in
//! an `a` tag and `30617` in a `k` tag, a follower is a kind 10018 that names
//! it in an `a` tag, and each key counts once however often it wrote. A
//! NIP-09 deletion removes an event from the store, so an un-star subtracts.

use nostr::event::{Event, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use nostr::types::Timestamp;
use relay_ui::ReadOnlyStore;
use relay_ui::chrome::{self, App, Apps, Profiles, ago, escape, icon, npub, percent_encode, svg};

use crate::lineage::ForkOf;
use crate::repo::{
    ANNOUNCEMENT, RepoAddress, STATE, authorized_authors, first_tag, head_branch, honoured_state,
    owner_announcement, repo_name, topics,
};

/// NIP-25 reaction: a star when it is `+` on a repo.
pub const REACTION: u16 = 7;
/// NIP-51 git repositories: the repos a key follows.
pub const FOLLOWED_REPOS: u16 = 10_018;
/// The most stars or followers one query reads.
const PEOPLE_LIMIT: usize = 5_000;

/// What a card links to: the apps that are on (an "Open repo" link needs
/// `/repo`), and the host visitors' clients reach this relay at, for the
/// `nostr://` clone URL, when the operator has said it.
#[derive(Debug, Clone, Default)]
pub struct Links {
    apps: Apps,
    relay_host: Option<String>,
}

impl Links {
    /// Links among `apps`, cloning through `relay_host` if it is known.
    pub fn new(apps: Apps, relay_host: Option<String>) -> Self {
        Self { apps, relay_host }
    }

    /// The apps that are on.
    pub fn apps(&self) -> &Apps {
        &self.apps
    }

    /// The relay's public host, if known.
    pub fn relay_host(&self) -> Option<&str> {
        self.relay_host.as_deref()
    }

    /// The repo's page on `/social`, if `/social` is on.
    pub fn social_path(&self, address: &RepoAddress) -> Option<String> {
        self.apps
            .serves(App::Social)
            .then(|| format!("{}/r/{}", App::Social.prefix(), address.path_segments()))
    }

    /// The repo's page on `/repo`, if `/repo` is on.
    pub fn repo_path(&self, address: &RepoAddress) -> Option<String> {
        self.apps.serves(App::Repo).then(|| address.repo_path())
    }

    /// Where a link to the repo goes: its social page, else its `/repo` page.
    pub fn repo_link(&self, address: &RepoAddress) -> Option<String> {
        self.social_path(address)
            .or_else(|| self.repo_path(address))
    }
}

/// A repository as a card shows it, loaded from the store.
#[derive(Debug, Clone)]
pub struct RepoCard {
    address: RepoAddress,
    announcement: Event,
    authorized: Vec<PublicKey>,
    profiles: Profiles,
    stars: usize,
    followers: usize,
    open_issues: usize,
    open_pulls: usize,
    /// The honoured state's time and default branch, if one is held.
    last_move: Option<(Timestamp, String)>,
    /// What the announcement's `u` tag says it was forked from, and the
    /// parent's name when this relay holds its announcement.
    fork_of: Option<(ForkOf, Option<String>)>,
}

impl RepoCard {
    /// The card for `address`, or none when the owner has announced no such
    /// repo on this relay.
    pub async fn load(store: &ReadOnlyStore, address: &RepoAddress) -> Option<Self> {
        let owner = *address.owner();
        let announcements = store
            .query(
                Filter::new()
                    .kind(Kind::from(ANNOUNCEMENT))
                    .author(owner)
                    .identifier(address.d()),
            )
            .await;
        let announcement = owner_announcement(&owner, address.d(), &announcements)?.clone();
        let authorized = authorized_authors(&announcement);
        let states = store
            .query(
                Filter::new()
                    .kind(Kind::from(STATE))
                    .authors(authorized.clone())
                    .identifier(address.d()),
            )
            .await;
        let last_move = honoured_state(&announcement, &states)
            .map(|state| (state.created_at, head_branch(state)));
        let stars = stargazers(store, address).await.len();
        let followers = followers(store, address).await.len();
        let (open_issues, open_pulls) =
            crate::threads::open_counts(store, &address.coordinate(), &authorized).await;
        let fork_of = match ForkOf::of(&announcement) {
            Some(parent) => {
                let parents = store
                    .query(
                        Filter::new()
                            .kind(Kind::from(ANNOUNCEMENT))
                            .author(*parent.address().owner())
                            .identifier(parent.address().d()),
                    )
                    .await;
                let name =
                    owner_announcement(parent.address().owner(), parent.address().d(), &parents)
                        .map(|parent| repo_name(parent).to_owned());
                Some((parent, name))
            }
            None => None,
        };
        let mut keys = authorized.clone();
        if let Some((parent, _)) = &fork_of
            && !keys.contains(parent.address().owner())
        {
            keys.push(*parent.address().owner());
        }
        let profiles = Profiles::load(store, &keys).await;
        Some(Self {
            address: address.clone(),
            announcement,
            authorized,
            profiles,
            stars,
            followers,
            open_issues,
            open_pulls,
            last_move,
            fork_of,
        })
    }

    /// A card read from `announcement` alone: no stars, followers, threads
    /// or ref moves, no profiles, and no parent name. For a reader that has
    /// nothing else, and for tests of what is drawn from a card.
    pub fn from_announcement(announcement: &Event) -> Option<Self> {
        let address = RepoAddress::of(announcement)?;
        Some(Self {
            address,
            announcement: announcement.clone(),
            authorized: authorized_authors(announcement),
            profiles: Profiles::default(),
            stars: 0,
            followers: 0,
            open_issues: 0,
            open_pulls: 0,
            last_move: None,
            fork_of: ForkOf::of(announcement).map(|parent| (parent, None)),
        })
    }

    /// The repo's address.
    pub fn address(&self) -> &RepoAddress {
        &self.address
    }

    /// The owner's latest announcement.
    pub fn announcement(&self) -> &Event {
        &self.announcement
    }

    /// The owner and the maintainers the owner named (ADR 0002): whose
    /// events about the repo are the maker's.
    pub fn authorized(&self) -> &[PublicKey] {
        &self.authorized
    }

    /// Whether `key` is the owner or a maintainer.
    pub fn is_maker(&self, key: &PublicKey) -> bool {
        self.authorized.contains(key)
    }

    /// The kind 0 profiles of the owner and maintainers.
    pub fn profiles(&self) -> &Profiles {
        &self.profiles
    }

    /// The repo's name.
    pub fn name(&self) -> &str {
        repo_name(&self.announcement)
    }

    /// The announcement's description, if it has one.
    pub fn description(&self) -> Option<&str> {
        first_tag(&self.announcement, "description")
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }

    /// Distinct people who starred it.
    pub fn stars(&self) -> usize {
        self.stars
    }

    /// Distinct people who follow it.
    pub fn followers(&self) -> usize {
        self.followers
    }

    /// Open issues, and open patch series and pull requests.
    pub fn open_threads(&self) -> (usize, usize) {
        (self.open_issues, self.open_pulls)
    }

    /// When an authorized author last moved a ref, and the default branch.
    pub fn last_move(&self) -> Option<(Timestamp, &str)> {
        self.last_move
            .as_ref()
            .map(|(at, branch)| (*at, branch.as_str()))
    }

    /// What the repo says it was forked from (NIP-34 `u` tag), if anything.
    pub fn fork_of(&self) -> Option<&ForkOf> {
        self.fork_of.as_ref().map(|(parent, _)| parent)
    }

    /// The parent's `owner / name`: its name when this relay holds its
    /// announcement, else its `d`; the owner by profile name.
    pub fn fork_title(&self) -> Option<String> {
        let (parent, name) = self.fork_of.as_ref()?;
        Some(format!(
            "{} / {}",
            chrome::shown_name(self.profiles.names(), parent.address().owner()),
            name.as_deref().unwrap_or(parent.address().d())
        ))
    }

    /// `owner / name`, the owner by profile name.
    pub fn title(&self) -> String {
        format!(
            "{} / {}",
            chrome::plain_name(self.profiles.names(), self.address.owner()),
            self.name()
        )
    }

    /// The card's HTML.
    pub fn render(&self, links: &Links) -> String {
        self.render_with_demo(links, None)
    }

    /// The card's HTML, with a Live demo button to `demo` when the repo has
    /// one (the app that draws the card finds it: NIP-5A sites are not
    /// `/repo`'s).
    pub fn render_with_demo(&self, links: &Links, demo: Option<&str>) -> String {
        let owner = self.address.owner();
        let href = links
            .social_path(&self.address)
            .or_else(|| links.repo_path(&self.address));
        let title = format!(
            "{} / <span class=\"strong\">{}</span>",
            escape(&chrome::plain_name(self.profiles.names(), owner)),
            escape(self.name())
        );
        let mut out = format!(
            "<div class=\"repo-card\" data-repo=\"{}\"><div class=\"title\">{}",
            escape(&self.address.coordinate()),
            svg(icon::REPO, ""),
        );
        match href {
            Some(href) => out.push_str(&format!("<a href=\"{}\">{title}</a>", escape(&href))),
            None => out.push_str(&format!("<span>{title}</span>")),
        }
        out.push_str("<span class=\"pill\">Public</span></div>");
        if let Some(title) = self.fork_title() {
            out.push_str(&format!(
                "<p class=\"forked-from small\">{}Forked from {}</p>",
                svg(icon::FORK, "icon-sm"),
                self.fork_of()
                    .and_then(|parent| links.repo_link(parent.address()))
                    .map(|href| format!("<a href=\"{}\">{}</a>", escape(&href), escape(&title)))
                    .unwrap_or_else(|| escape(&title)),
            ));
        }
        if let Some(description) = self.description() {
            out.push_str(&format!("<p>{}</p>", escape(description)));
        }
        let topics = topics(&self.announcement);
        if !topics.is_empty() {
            out.push_str("<div class=\"chips\">");
            for topic in topics {
                out.push_str(&format!(
                    "<span class=\"chip chip-topic\">{}</span>",
                    escape(topic)
                ));
            }
            out.push_str("</div>");
        }
        let (issues, pulls) = self.open_threads();
        out.push_str(&format!(
            "<div class=\"meta\"><span data-stars=\"{0}\">{1}{0} {2}</span>\
             <span data-followers=\"{3}\">{4}{3} {5}</span>\
             <span>{issues} open {6}</span><span>{pulls} open {7}</span>",
            self.stars,
            svg(icon::STAR, "icon-sm"),
            plural(self.stars, "star", "stars"),
            self.followers,
            svg(icon::EYE, "icon-sm"),
            plural(self.followers, "follower", "followers"),
            plural(issues, "issue", "issues"),
            plural(pulls, "pull request", "pull requests"),
        ));
        if let Some((at, branch)) = self.last_move() {
            out.push_str(&format!(
                "<span>{}{} moved {}</span>",
                svg(icon::BRANCH, "icon-sm"),
                escape(branch),
                ago(at)
            ));
        }
        out.push_str("</div><div class=\"clone\">");
        let nip05 = self.profiles.nip05(owner);
        for (url, claimed) in clone_urls(&self.address, links.relay_host(), nip05) {
            out.push_str(&format!(
                "<code class=\"copy-field\">git clone {}</code>",
                escape(&url)
            ));
            if claimed {
                out.push_str(
                    "<span class=\"muted small\">The NIP-05 name is the owner's claim; \
                     this relay has not checked it.</span>",
                );
            }
        }
        out.push_str("</div>");
        let mut buttons = String::new();
        if let Some(url) = demo {
            buttons.push_str(&format!(
                "<a class=\"btn btn-success\" href=\"{}\" rel=\"noopener noreferrer\" data-demo>{}Live demo</a>",
                escape(url),
                svg(icon::PLAY, "icon-sm")
            ));
        }
        if let Some(path) = links.repo_path(&self.address) {
            buttons.push_str(&format!(
                "<a class=\"btn\" href=\"{}\">{}Open repo</a>",
                escape(&path),
                svg(icon::CODE, "icon-sm")
            ));
        }
        if let Some(path) = links.social_path(&self.address) {
            buttons.push_str(&format!(
                "<a class=\"btn\" href=\"{}/stars\">{}Stars</a>",
                escape(&path),
                svg(icon::STAR, "icon-sm")
            ));
        }
        if !buttons.is_empty() {
            out.push_str(&format!("<div class=\"links\">{buttons}</div>"));
        }
        out.push_str("</div>");
        out
    }
}

/// `singular` for one, else `plural`.
pub fn plural<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 { singular } else { plural }
}

/// The `nostr://` URLs a git remote helper clones `address` from, each with
/// whether it rests on an unverified NIP-05 claim: by npub, then by the
/// owner's NIP-05 name when the profile claims one. With the relay's public
/// host known it is the relay hint, `nostr://<npub>/<host>/<d>`; without it,
/// `nostr://<npub>/<d>`. A pure function.
pub fn clone_urls(
    address: &RepoAddress,
    relay_host: Option<&str>,
    nip05: Option<&str>,
) -> Vec<(String, bool)> {
    let d = percent_encode(address.d());
    let url = |who: &str| match relay_host {
        Some(host) => format!("nostr://{who}/{host}/{d}"),
        None => format!("nostr://{who}/{d}"),
    };
    let mut urls = vec![(url(&npub(address.owner())), false)];
    if let Some(nip05) = nip05.and_then(chrome::nip05_identifier) {
        urls.push((url(&nip05), true));
    }
    urls
}

/// Whether `event` names `coordinate` in a tag called `name` (relay hints
/// and markers after the value are ignored).
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

/// The people who starred `address`, newest first.
pub async fn stargazers(
    store: &ReadOnlyStore,
    address: &RepoAddress,
) -> Vec<(PublicKey, Timestamp)> {
    let coordinate = address.coordinate();
    let events = store
        .query(
            Filter::new()
                .kind(Kind::from(REACTION))
                .custom_tag(SingleLetterTag::LOWERCASE_A, coordinate.clone())
                .limit(PEOPLE_LIMIT),
        )
        .await;
    people(&events, |event| is_star(event, &coordinate))
}

/// The people whose followed-repos list (kind 10018) names `address`,
/// newest first.
pub async fn followers(
    store: &ReadOnlyStore,
    address: &RepoAddress,
) -> Vec<(PublicKey, Timestamp)> {
    let coordinate = address.coordinate();
    let events = store
        .query(
            Filter::new()
                .kind(Kind::from(FOLLOWED_REPOS))
                .custom_tag(SingleLetterTag::LOWERCASE_A, coordinate.clone())
                .limit(PEOPLE_LIMIT),
        )
        .await;
    people(&events, |event| {
        event.kind == Kind::from(FOLLOWED_REPOS) && names_value(event, "a", &coordinate)
    })
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;

    use super::*;

    fn signed(keys: &Keys, kind: u16, at: u64, content: &str, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(kind), content)
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .custom_created_at(Timestamp::from(at))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn a_star_is_a_plus_on_the_repo_with_its_kind() {
        let owner = Keys::generate().public_key();
        let a = format!("30617:{}:r", owner.to_hex());
        let fan = Keys::generate();
        let star = |content: &str, tags: &[&[&str]]| signed(&fan, 7, 1, content, tags);
        assert!(is_star(
            &star("+", &[&["a", &a, "wss://x"], &["k", "30617"]]),
            &a
        ));
        assert!(!is_star(&star("-", &[&["a", &a], &["k", "30617"]]), &a));
        assert!(!is_star(&star("🔥", &[&["a", &a], &["k", "30617"]]), &a));
        assert!(!is_star(&star("+", &[&["a", &a]]), &a));
        assert!(!is_star(&star("+", &[&["e", &a], &["k", "30617"]]), &a));
        let other = format!("30617:{}:other", owner.to_hex());
        assert!(!is_star(&star("+", &[&["a", &other], &["k", "30617"]]), &a));
    }

    #[test]
    fn people_are_counted_once_however_often_they_write() {
        let a = Keys::generate();
        let b = Keys::generate();
        let events = [
            signed(&a, 7, 10, "+", &[]),
            signed(&a, 7, 30, "+", &[]),
            signed(&b, 7, 20, "+", &[]),
            signed(&b, 7, 40, "-", &[]),
        ];
        let counted = people(&events, |event| event.content == "+");
        assert_eq!(
            counted,
            vec![
                (a.public_key(), Timestamp::from(30)),
                (b.public_key(), Timestamp::from(20)),
            ]
        );
    }

    #[test]
    fn the_clone_url_names_the_relay_only_when_it_is_known() {
        let owner = Keys::generate().public_key();
        let address = RepoAddress::new(owner, "my repo").unwrap();
        let npub = npub(&owner);
        assert_eq!(
            clone_urls(&address, None, None),
            vec![(format!("nostr://{npub}/my%20repo"), false)]
        );
        assert_eq!(
            clone_urls(&address, Some("relay.example"), Some("Maya@Tide.dev")),
            vec![
                (format!("nostr://{npub}/relay.example/my%20repo"), false),
                (
                    "nostr://maya@tide.dev/relay.example/my%20repo".to_owned(),
                    true
                ),
            ]
        );
        // A claim that would break the URL is not used.
        assert_eq!(clone_urls(&address, None, Some("a/b@c.d")).len(), 1);
    }
}
