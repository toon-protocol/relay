//! A builder's profile, `/social/p/<npub>` (proposals 4 and 10 of
//! `docs/social-for-builders-ux.md`): who the key says it is, the repos it
//! keeps, what it starred and wrote, and a year of its activity. One route
//! per tab, so the page needs no script:
//!
//! | Tab          | Path       | What it lists                                                      |
//! | ------------ | ---------- | ------------------------------------------------------------------ |
//! | Pinned       | (none)     | the key's kind 10617 pins, the activity graph, recent activity     |
//! | Repositories | `/repos`   | repos it announced (30617) and repos whose owner named it maintainer |
//! | Starred      | `/starred` | repos it starred (kind 7 `+`, `a`, `k=30617`)                      |
//! | Posts        | `/posts`   | its notes that start a conversation (kind 1) and articles (30023) |
//!
//! The sidebar on every tab reads the key's kind 0 (with the NIP-24
//! extras), NIP-38 statuses, NIP-39 identities, its kind 3 and 10018 lists,
//! who follows it, and its NIP-58 badges. Every identity claim is drawn as a
//! claim: the relay checks none of them (ADR 0001).
//!
//! There is no "agents this person runs" section: no NIP lets a key say
//! which other keys it operates in a way a reader could check, so a page
//! that listed them would be guessing. A key whose own profile says
//! `bot: true` is marked as one.

mod activity;
mod badges;
mod identity;

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use nostr::event::{Event, EventId, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use nostr::types::Timestamp;
use relay_ui::chrome::{self, Profiles, ago, avatar, date, escape, icon, name, npub, svg};
use ui_nip34::card::{self, Links, RepoCard, plural};
use ui_nip34::repo::{ANNOUNCEMENT, RepoAddress, authorized_authors, first_tag, newest, topics};

use crate::kinds::{ARTICLE, FOLLOWED_REPOS, FOLLOWS, NOTE, REACTION};
use crate::mute::Mutes;
use crate::post::{self, View};
use crate::{PREFIX, Pages, content, html, load_cards, not_found, parse_pubkey};
use activity::Graph;
use badges::Badge;
use identity::{Identity, Metadata, Status};

/// GitWorkshop's pinned repositories list: ordered `a` tags to the key's
/// own repos. Not in the NIPs (`docs/social-for-builders-ux.md` §8).
const PINNED_REPOS: u16 = 10_617;
/// The most repos a pinned list shows, as GitHub does.
const PINS: usize = 6;
/// The most announcements scanned for a `maintainers` tag naming the key.
const MAINTAINER_SCAN: usize = 2_000;
/// The most events one of the page's lists reads.
const LIMIT: usize = 1_000;
/// The most events the activity graph reads.
const ACTIVITY_LIMIT: usize = 5_000;
/// The most follow lists read to count followers.
const FOLLOWER_LIMIT: usize = 5_000;
/// The most rows of recent activity.
const RECENT: usize = 10;
/// The most posts the Posts tab shows.
const POSTS: usize = 40;

/// A tab of the profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Pinned,
    Repositories,
    Starred,
    Posts,
}

impl Tab {
    const ALL: [Tab; 4] = [Tab::Pinned, Tab::Repositories, Tab::Starred, Tab::Posts];

    /// The tab a path segment names; Pinned has none.
    fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|tab| *tab != Tab::Pinned && tab.slug() == slug)
    }

    fn slug(self) -> &'static str {
        match self {
            Self::Pinned => "",
            Self::Repositories => "repos",
            Self::Starred => "starred",
            Self::Posts => "posts",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Pinned => "Pinned",
            Self::Repositories => "Repositories",
            Self::Starred => "Starred",
            Self::Posts => "Posts",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Pinned => icon::PERSON,
            Self::Repositories => icon::REPO,
            Self::Starred => icon::STAR,
            Self::Posts => icon::COMMENT,
        }
    }
}

/// How a key relates to a repo it keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Role {
    Owner,
    Maintainer,
}

impl Role {
    fn label(self) -> &'static str {
        match self {
            Self::Owner => "owner",
            Self::Maintainer => "maintainer",
        }
    }
}

/// The repos `key` keeps among `announcements`: the ones it announced, then
/// the ones whose owner's newest announcement names it in `maintainers`
/// (ADR 0002), each once, newest first within each. A pure function.
fn kept_repos(key: &PublicKey, announcements: &[Event]) -> Vec<(RepoAddress, Role)> {
    let mut current: HashMap<RepoAddress, &Event> = HashMap::new();
    for announcement in announcements {
        let Some(address) = RepoAddress::of(announcement) else {
            continue;
        };
        if announcement.pubkey != *address.owner() {
            continue;
        }
        match current.get(&address) {
            Some(held) if !ui_nip34::repo::supersedes(announcement, held) => {}
            _ => {
                current.insert(address, announcement);
            }
        }
    }
    let mut kept: Vec<(RepoAddress, Role, Timestamp)> = current
        .into_iter()
        .filter_map(|(address, announcement)| {
            let role = if address.owner() == key {
                Role::Owner
            } else if authorized_authors(announcement).contains(key) {
                Role::Maintainer
            } else {
                return None;
            };
            Some((address, role, announcement.created_at))
        })
        .collect();
    kept.sort_by(|(a, role_a, at_a), (b, role_b, at_b)| {
        (*role_a == Role::Maintainer)
            .cmp(&(*role_b == Role::Maintainer))
            .then(at_b.cmp(at_a))
            .then_with(|| a.coordinate().cmp(&b.coordinate()))
    });
    kept.into_iter()
        .map(|(address, role, _)| (address, role))
        .collect()
}

/// The repos `key` starred among `reactions`, newest first, each once. A
/// pure function.
fn starred(key: &PublicKey, reactions: &[Event]) -> Vec<RepoAddress> {
    let mut reactions: Vec<&Event> = reactions
        .iter()
        .filter(|event| event.pubkey == *key)
        .collect();
    reactions.sort_by(|a, b| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
    let mut found: Vec<RepoAddress> = Vec::new();
    for event in reactions {
        for repo in RepoAddress::tagged(event, "a") {
            if card::is_star(event, &repo.coordinate()) && !found.contains(&repo) {
                found.push(repo);
            }
        }
    }
    found
}

/// The repos `key`'s newest pinned list names, in its order, that `key`
/// keeps, at most [`PINS`]. A pure function.
fn pinned(key: &PublicKey, lists: &[Event], kept: &[(RepoAddress, Role)]) -> Vec<RepoAddress> {
    let Some(list) = newest(
        lists
            .iter()
            .filter(|event| event.pubkey == *key && event.kind == Kind::from(PINNED_REPOS)),
    ) else {
        return Vec::new();
    };
    RepoAddress::tagged(list, "a")
        .into_iter()
        .filter(|repo| kept.iter().any(|(held, _)| held == repo))
        .take(PINS)
        .collect()
}

/// The distinct keys whose follow list names `key`, other than `key`.
fn followers(key: &PublicKey, lists: &[Event]) -> usize {
    let hex = key.to_hex();
    lists
        .iter()
        .filter(|list| list.kind == Kind::from(FOLLOWS) && list.pubkey != *key)
        .filter(|list| {
            list.tags
                .iter()
                .map(|tag| tag.as_slice())
                .any(|tag| matches!(tag, [name, value, ..] if name == "p" && *value == hex))
        })
        .map(|list| list.pubkey)
        .collect::<HashSet<_>>()
        .len()
}

/// The distinct values of `name` tags on `key`'s newest list of `kind`.
fn list_size(key: &PublicKey, lists: &[Event], kind: u16, name: &str) -> usize {
    newest(
        lists
            .iter()
            .filter(|event| event.pubkey == *key && event.kind == Kind::from(kind)),
    )
    .map(|list| {
        list.tags
            .iter()
            .map(|tag| tag.as_slice())
            .filter_map(|tag| match tag {
                [n, value, ..] if n == name => Some(value.as_str()),
                _ => None,
            })
            .collect::<HashSet<_>>()
            .len()
    })
    .unwrap_or(0)
}

/// What every tab's sidebar shows.
struct Who {
    key: PublicKey,
    meta: Metadata,
    statuses: Vec<Status>,
    identities: Vec<Identity>,
    followers: usize,
    following: usize,
    repos_followed: usize,
    badges: Vec<Badge>,
    /// Whether the store holds anything this key signed.
    known: bool,
}

pub(crate) async fn overview(
    State(pages): State<Pages>,
    Path(target): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    page(&pages, &target, Tab::Pinned, query.as_deref()).await
}

pub(crate) async fn tab(
    State(pages): State<Pages>,
    Path((target, tab)): Path<(String, String)>,
    RawQuery(query): RawQuery,
) -> Response {
    match Tab::from_slug(&tab) {
        Some(tab) => page(&pages, &target, tab, query.as_deref()).await,
        None => not_found(&pages),
    }
}

async fn page(pages: &Pages, target: &str, tab: Tab, query: Option<&str>) -> Response {
    let Some(key) = parse_pubkey(target) else {
        return not_found(pages);
    };
    let store = &pages.store;
    // `?as=<npub>`: read through that key's mute list (NIP-51 kind 10000).
    let lens = crate::viewer::as_key(query);
    let mutes = match &lens {
        Some(viewer) => Mutes::load(store, viewer).await,
        None => Mutes::default(),
    };
    let now = chrome::now();
    let lists = store
        .query(
            Filter::new()
                .author(key)
                .kinds(
                    [
                        0,
                        FOLLOWS,
                        FOLLOWED_REPOS,
                        identity::EXTERNAL_IDENTITIES,
                        badges::PROFILE_BADGES,
                        badges::LEGACY_PROFILE_BADGES,
                        identity::USER_STATUS,
                        PINNED_REPOS,
                    ]
                    .map(Kind::from),
                )
                .limit(LIMIT),
        )
        .await;
    let who = who(pages, key, &lists, now).await;

    let mut announcements = store
        .query(
            Filter::new()
                .kind(Kind::from(ANNOUNCEMENT))
                .author(key)
                .limit(LIMIT),
        )
        .await;
    announcements.extend(
        store
            .query(
                Filter::new()
                    .kind(Kind::from(ANNOUNCEMENT))
                    .limit(MAINTAINER_SCAN),
            )
            .await,
    );
    let kept = kept_repos(&key, &announcements);
    let reactions = store
        .query(
            Filter::new()
                .kind(Kind::from(REACTION))
                .author(key)
                .custom_tag(SingleLetterTag::LOWERCASE_K, ANNOUNCEMENT.to_string())
                .limit(LIMIT),
        )
        .await;
    let mut stars = starred(&key, &reactions);
    let starred_all = stars.len();
    stars.retain(|repo| !mutes.hides_key(repo.owner()));
    let starred_muted = starred_all - stars.len();

    let links = pages.links();
    let mut body = banner(&who.meta);
    body.push_str("<div class=\"profile\">");
    let mut issuers: Vec<PublicKey> = who.badges.iter().map(|badge| badge.issuer).collect();
    issuers.push(key);
    let mut main = String::new();
    let base = format!("{PREFIX}/p/{}", npub(&key));
    let tabs: Vec<chrome::Tab> = Tab::ALL
        .into_iter()
        .map(|each| {
            let lens_query = crate::viewer::as_query(lens.as_ref(), true);
            let href = if each == Tab::Pinned {
                format!("{base}{lens_query}")
            } else {
                format!("{base}/{}{lens_query}", each.slug())
            };
            let made = chrome::Tab::new(each.label(), each.icon(), href).current(each == tab);
            match each {
                Tab::Repositories => made.count(kept.len()),
                Tab::Starred => made.count(stars.len()),
                _ => made,
            }
        })
        .collect();
    main.push_str(&chrome::tabs("Profile", &tabs));
    let mut names_wanted = issuers;
    names_wanted.extend(lens);
    let lens_names = Profiles::load(store, &lens.into_iter().collect::<Vec<_>>()).await;
    if let Some(viewer) = &lens {
        main.push_str(&format!(
            "<p class=\"muted small\" data-muted>Read as {}: {}{starred_muted} starred {} by \
             people their mute list (NIP-51 kind 10000) names left out.</p>",
            name(lens_names.names(), viewer),
            if mutes.hides_key(&key) {
                "their mute list names this person, so their posts are left out of that key's \
                 feeds; "
            } else {
                ""
            },
            plural(starred_muted, "repo", "repos"),
        ));
    }
    match tab {
        Tab::Pinned => {
            let pins = pinned(&key, &lists, &kept);
            let (shown, note) = if pins.is_empty() {
                let owned: Vec<RepoAddress> = kept
                    .iter()
                    .filter(|(_, role)| *role == Role::Owner)
                    .map(|(repo, _)| repo.clone())
                    .collect();
                let cards = load_cards(store, owned.clone()).await;
                let mut ranked: Vec<RepoAddress> = owned
                    .into_iter()
                    .filter(|repo| cards.contains_key(repo))
                    .collect();
                ranked.sort_by_key(|repo| std::cmp::Reverse(cards[repo].stars()));
                ranked.truncate(PINS);
                (
                    ranked,
                    "No pinned list (kind 10617), so these are the most-starred repos this key owns.",
                )
            } else {
                (pins, "Pinned by this key, in its order (kind 10617).")
            };
            let cards = load_cards(store, shown.clone()).await;
            for card in cards.values() {
                names_wanted.extend(card.authorized().iter().copied());
            }
            main.push_str(&format!(
                "<section class=\"profile-section\"><div class=\"section-head\"><h2>Pinned</h2>\
                 <span class=\"muted small\">{note}</span></div>"
            ));
            main.push_str(&tiles(
                &links,
                &cards,
                &shown,
                &kept,
                "Nothing to pin yet: this key keeps no repos here.",
            ));
            main.push_str("</section>");
            main.push_str(&activity(pages, &links, &key, now).await);
        }
        Tab::Repositories => {
            let addresses: Vec<RepoAddress> = kept.iter().map(|(repo, _)| repo.clone()).collect();
            let cards = load_cards(store, addresses.clone()).await;
            main.push_str(&format!(
                "<p class=\"muted small\">{} {} it announced (kind 30617), then {} whose owner named \
                 it a maintainer (ADR 0002).</p>",
                kept.iter().filter(|(_, role)| *role == Role::Owner).count(),
                plural(
                    kept.iter().filter(|(_, role)| *role == Role::Owner).count(),
                    "repo",
                    "repos"
                ),
                kept.iter()
                    .filter(|(_, role)| *role == Role::Maintainer)
                    .count(),
            ));
            main.push_str(&tiles(
                &links,
                &cards,
                &addresses,
                &kept,
                "This key keeps no repos on this relay.",
            ));
        }
        Tab::Starred => {
            let cards = load_cards(store, stars.clone()).await;
            main.push_str(
                "<p class=\"muted small\">Repos this key starred: a <code>+</code> reaction on the \
                 repo's address, each a paid write. Newest first.</p>",
            );
            main.push_str(&tiles(
                &links,
                &cards,
                &stars,
                &kept,
                "This key has starred no repo here.",
            ));
        }
        Tab::Posts => {
            main.push_str(&posts(pages, &links, &key).await);
        }
    }

    let profiles = Profiles::load(store, &names_wanted).await;
    body.push_str(&sidebar(&who, profiles.names()));
    body.push_str(&format!("<div class=\"profile-main\">{main}</div></div>"));
    let title = who
        .meta
        .name
        .clone()
        .unwrap_or_else(|| format!("{}…", &npub(&key)[..12]));
    html(
        pages,
        StatusCode::OK,
        &format!("{title} · {}", tab.label()),
        None,
        &body,
    )
}

/// The sidebar's facts about `key`, read from `lists` (everything the key
/// signed of the list kinds) and the store.
async fn who(pages: &Pages, key: PublicKey, lists: &[Event], now: i64) -> Who {
    let store = &pages.store;
    let meta = identity::metadata(&key, lists);
    let mut identities = newest(lists.iter().filter(|event| {
        event.pubkey == key && event.kind == Kind::from(identity::EXTERNAL_IDENTITIES)
    }))
    .map(identity::identities)
    .unwrap_or_default();
    if identities.is_empty() {
        identities = meta.identities.clone();
    }
    let follow_lists = store
        .query(
            Filter::new()
                .kind(Kind::from(FOLLOWS))
                .custom_tag(SingleLetterTag::LOWERCASE_P, key.to_hex())
                .limit(FOLLOWER_LIMIT),
        )
        .await;
    let pairs = badges::accepted(&key, lists);
    let shown = if pairs.is_empty() {
        Vec::new()
    } else {
        let awards = store
            .query(Filter::new().ids(pairs.iter().map(|(_, id)| *id)))
            .await;
        let issuers: Vec<PublicKey> = awards.iter().map(|award| award.pubkey).collect();
        let definitions = if issuers.is_empty() {
            Vec::new()
        } else {
            store
                .query(
                    Filter::new()
                        .kind(Kind::from(badges::DEFINITION))
                        .authors(issuers)
                        .limit(LIMIT),
                )
                .await
        };
        badges::shown(&key, &pairs, &awards, &definitions)
    };
    Who {
        key,
        statuses: identity::statuses(&key, lists, now),
        identities,
        followers: followers(&key, &follow_lists),
        following: list_size(&key, lists, FOLLOWS, "p"),
        repos_followed: list_size(&key, lists, FOLLOWED_REPOS, "a"),
        badges: shown,
        known: !lists.is_empty(),
        meta,
    }
}

/// The banner: the profile's `https` image, which the visitor's browser
/// loads, or a plain band.
fn banner(meta: &Metadata) -> String {
    match &meta.banner {
        Some(image) => format!(
            "<div class=\"profile-banner\">{}</div>",
            image.html("banner")
        ),
        None => "<div class=\"profile-banner empty\"></div>".to_owned(),
    }
}

/// The left column: the person, their claims, counts and badges.
fn sidebar(who: &Who, names: &HashMap<PublicKey, String>) -> String {
    let key = &who.key;
    let npub = npub(key);
    let shown = who
        .meta
        .name
        .clone()
        .unwrap_or_else(|| format!("{}…", &npub[..12]));
    let mut out = format!(
        "<aside class=\"profile-side\"><div class=\"profile-avatar\">{}</div><div>\
         <h1 class=\"profile-name\">{}{}</h1>",
        avatar(&shown, false),
        escape(&shown),
        if who.meta.bot {
            " <span class=\"chip chip-bot\" title=\"Its profile says its posts are wholly or partly \
             automated (NIP-24 bot)\">bot</span>"
        } else {
            ""
        },
    );
    if let Some(handle) = &who.meta.handle {
        out.push_str(&format!(
            "<div class=\"muted profile-handle\">{}</div>",
            escape(handle)
        ));
    }
    if let Some(nip05) = &who.meta.nip05 {
        out.push_str(&format!(
            "<div class=\"profile-nip05\"><span>{}</span><span class=\"chip\" title=\"A NIP-05 \
             name is the profile's claim. This relay never fetches the .well-known document that \
             would prove it.\">unverified</span></div>",
            escape(nip05)
        ));
    }
    out.push_str(&format!(
        "<code class=\"muted small truncate profile-npub\" title=\"{0}\">{0}</code></div>",
        escape(&npub)
    ));
    if !who.known {
        out.push_str(
            "<p class=\"muted small\">This relay holds no profile or list for this key.</p>",
        );
    }
    for status in &who.statuses {
        out.push_str(&status_html(status));
    }
    if let Some(about) = &who.meta.about {
        out.push_str(&format!(
            "<p class=\"text profile-about\">{}</p>",
            content::render(about, &[], names).html.trim()
        ));
    }
    out.push_str(&format!(
        "<div class=\"profile-follow\"><a class=\"btn btn-lg\" href=\"nostr:{0}\">{1}Follow in your client</a>\
         <span class=\"muted small\">Opens your Nostr client. A follow is a paid write; this page writes nothing.</span></div>\
         <div class=\"tally\"><span><span class=\"strong\" data-followers=\"{2}\">{2}</span> {3}</span>\
         <a href=\"{PREFIX}/following/{0}\"><span class=\"strong\" data-following=\"{4}\">{4}</span> following</a>\
         <span><span class=\"strong\" data-repos-followed=\"{5}\">{5}</span> {6} followed</span></div>",
        escape(&npub),
        svg(icon::PERSON, "icon-sm"),
        who.followers,
        plural(who.followers, "follower", "followers"),
        who.following,
        who.repos_followed,
        plural(who.repos_followed, "repo", "repos"),
    ));
    if who.meta.website.is_some() || !who.identities.is_empty() {
        out.push_str("<section class=\"profile-links\"><h2>Also on</h2><ul class=\"facts\">");
        if let Some(website) = &who.meta.website {
            out.push_str(&format!(
                "<li>{}<a class=\"truncate\" href=\"{1}\" rel=\"nofollow noopener noreferrer\">{2}</a></li>",
                svg(icon::LINK, ""),
                escape(website),
                escape(
                    website
                        .trim_start_matches("https://")
                        .trim_start_matches("http://")
                        .trim_end_matches('/')
                ),
            ));
        }
        for claim in &who.identities {
            out.push_str(&identity_html(claim));
        }
        out.push_str(
            "</ul><p class=\"muted small\">Accounts the key claims (NIP-39). This relay checks none \
             of them; open the proof to check one yourself.</p></section>",
        );
    }
    if !who.badges.is_empty() {
        out.push_str("<section class=\"profile-badges\"><h2>Badges</h2><div class=\"badges\">");
        for badge in &who.badges {
            let issuer = chrome::plain_name(names, &badge.issuer);
            out.push_str(&format!(
                "<a class=\"badge\" href=\"{}\" title=\"{}\">{}</a>",
                chrome::profile_path(&badge.issuer),
                escape(&format!(
                    "{}{}Awarded by {issuer} on {}",
                    badge.description.as_deref().unwrap_or(""),
                    if badge.description.is_some() {
                        ". "
                    } else {
                        ""
                    },
                    date(i64::try_from(badge.awarded_at).unwrap_or(0)),
                )),
                escape(&badge.name),
            ));
        }
        out.push_str(
            "</div><p class=\"muted small\">Badges the key accepted (NIP-58), each awarded by the key \
             that defined it. Select one to see its issuer.</p></section>",
        );
    }
    out.push_str("</aside>");
    out
}

/// A NIP-38 status line.
fn status_html(status: &Status) -> String {
    let mut text = escape(&status.text);
    if let Some(repo) = &status.repo {
        text.push_str(&format!(
            " <a class=\"mono\" href=\"{PREFIX}/r/{}\">{}</a>",
            escape(&repo.path_segments()),
            escape(repo.d())
        ));
    }
    if let Some(link) = &status.link {
        text.push_str(&format!(
            " <a href=\"{}\" rel=\"nofollow noopener noreferrer\">{}</a>",
            escape(link),
            svg(icon::LINK, "icon-sm")
        ));
    }
    let kind = if status.kind == "general" {
        String::new()
    } else {
        format!(
            "<span class=\"muted small\">{}</span> ",
            escape(&status.kind)
        )
    };
    format!(
        "<div class=\"status-card\" data-status=\"{}\"><span class=\"dot\" aria-hidden=\"true\"></span><span>{kind}{text}</span></div>",
        escape(&status.kind)
    )
}

/// A NIP-39 identity, linked where the platform has a page, with its proof.
fn identity_html(claim: &Identity) -> String {
    let account = match claim.profile_url() {
        Some(url) => format!(
            "<a href=\"{}\" rel=\"nofollow noopener noreferrer\">{}</a>",
            escape(&url),
            escape(&claim.identity)
        ),
        None => format!("<span>{}</span>", escape(&claim.identity)),
    };
    let proof = match claim.proof_url() {
        Some(url) => format!(
            "<a class=\"chip\" href=\"{}\" rel=\"nofollow noopener noreferrer\">proof, unchecked</a>",
            escape(&url)
        ),
        None => "<span class=\"chip\">no proof</span>".to_owned(),
    };
    format!(
        "<li data-identity=\"{}\"><span class=\"strong\">{}</span> {account} {proof}</li>",
        escape(&claim.platform),
        escape(&claim.label())
    )
}

/// Repo tiles for `repos`, in order, each with the key's role in it.
fn tiles(
    links: &Links,
    cards: &HashMap<RepoAddress, RepoCard>,
    repos: &[RepoAddress],
    kept: &[(RepoAddress, Role)],
    empty: &str,
) -> String {
    let drawn: Vec<String> = repos
        .iter()
        .filter_map(|repo| cards.get(repo))
        .map(|card| {
            let repo = card.address();
            let role = kept
                .iter()
                .find(|(held, _)| held == repo)
                .map(|(_, role)| format!("<span class=\"chip\">{}</span>", role.label()))
                .unwrap_or_default();
            let href = links.social_path(repo).unwrap_or_else(|| repo.repo_path());
            let mut tile = format!(
                "<article class=\"repo-tile\" data-repo=\"{}\"><div class=\"title\">{}\
                 <a href=\"{}\">{}</a>{role}</div>",
                escape(&repo.coordinate()),
                svg(icon::REPO, ""),
                escape(&href),
                escape(&card.title()),
            );
            if let Some(description) = card.description() {
                tile.push_str(&format!(
                    "<p>{}</p>",
                    escape(&content::clip(description, 160))
                ));
            }
            tile.push_str("<div class=\"meta\">");
            if let Some(topic) = topics(card.announcement()).first() {
                tile.push_str(&format!(
                    "<span class=\"chip chip-topic\">{}</span>",
                    escape(topic)
                ));
            }
            tile.push_str(&format!(
                "<span data-stars=\"{0}\">{1}{0}</span><span>{2}{3} {4}</span></div></article>",
                card.stars(),
                svg(icon::STAR, "icon-sm"),
                svg(icon::EYE, "icon-sm"),
                card.followers(),
                plural(card.followers(), "follower", "followers"),
            ));
            tile
        })
        .collect();
    if drawn.is_empty() {
        format!("<div class=\"box\"><div class=\"empty\">{empty}</div></div>")
    } else {
        format!("<div class=\"repo-grid\">{}</div>", drawn.concat())
    }
}

/// The activity graph and the recent activity under it.
async fn activity(pages: &Pages, links: &Links, key: &PublicKey, now: i64) -> String {
    let store = &pages.store;
    let since = u64::try_from(activity::since(now)).unwrap_or(0);
    let mut events = store
        .query(
            Filter::new()
                .author(*key)
                .kinds(activity::COUNTED.map(Kind::from))
                .since(Timestamp::from(since))
                .limit(ACTIVITY_LIMIT),
        )
        .await;
    events.sort_by(|a, b| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
    let graph = activity::graph(key, &events, now);
    let mut out = format!(
        "<section class=\"profile-section\" data-contributions=\"{0}\" data-repos-touched=\"{2}\">\
         <h2>{0} {1} across {2} {3} in the last year</h2><div class=\"box contrib\">",
        graph.total,
        plural(graph.total, "contribution", "contributions"),
        graph.repos,
        plural(graph.repos, "repo", "repos"),
    );
    out.push_str(&grid(&graph));
    out.push_str(&format!(
        "<div class=\"contrib-foot\"><p class=\"muted small\">{}</p>\
         <span class=\"legend muted small\">Less <span class=\"cell l0\"></span><span class=\"cell l1\"></span>\
         <span class=\"cell l2\"></span><span class=\"cell l3\"></span><span class=\"cell l4\"></span> More</span></div></div></section>",
        activity::RULE
    ));
    let recent: Vec<&Event> = events.iter().take(RECENT).collect();
    let target_ids: Vec<EventId> = recent
        .iter()
        .filter(|event| {
            activity::is_status(event.kind) || event.kind.as_u16() == activity::PULL_UPDATE
        })
        .filter_map(|event| target_id(event))
        .collect();
    let targets: HashMap<EventId, Event> = if target_ids.is_empty() {
        HashMap::new()
    } else {
        store
            .query(Filter::new().ids(target_ids))
            .await
            .into_iter()
            .map(|event| (event.id, event))
            .collect()
    };
    out.push_str("<section class=\"profile-section\"><h2>Recent activity</h2>");
    if recent.is_empty() {
        out.push_str(
            "<div class=\"box\"><div class=\"empty\">No activity in the last year.</div></div>",
        );
    } else {
        out.push_str("<ul class=\"box rows activity\">");
        for event in recent {
            out.push_str(&activity_row(links, event, &targets));
        }
        out.push_str("</ul>");
    }
    out.push_str("</section>");
    out
}

/// The 53 × 7 squares, a week a column.
fn grid(graph: &Graph) -> String {
    let mut out = String::from(
        "<div class=\"contrib-scroll\"><div class=\"contrib-grid\" role=\"img\" aria-label=\"Contributions per day over the last year\">",
    );
    for square in graph.squares() {
        match square {
            Some((day, count, shade)) => out.push_str(&format!(
                "<span class=\"cell l{shade}\" title=\"{count} {} on {}\"></span>",
                plural(count, "contribution", "contributions"),
                date(day)
            )),
            None => out.push_str("<span class=\"cell future\"></span>"),
        }
    }
    out.push_str("</div></div>");
    out
}

/// The event a status or pull request update is about: its `e` tag marked
/// `root`, else its first `e` tag.
fn target_id(event: &Event) -> Option<EventId> {
    let tags: Vec<&[String]> = event.tags.iter().map(|tag| tag.as_slice()).collect();
    tags.iter()
        .find_map(|tag| match tag {
            [name, id, _, marker, ..] if name == "e" && marker == "root" => {
                EventId::from_hex(id).ok()
            }
            _ => None,
        })
        .or_else(|| {
            tags.iter().find_map(|tag| match tag {
                [name, id, ..] if name == "e" => EventId::from_hex(id).ok(),
                _ => None,
            })
        })
}

/// An issue's, patch's or pull request's title: its `subject`, else its
/// first line.
fn subject(event: &Event) -> String {
    first_tag(event, "subject")
        .map(str::to_owned)
        .unwrap_or_else(|| content::first_line(&event.content, 80))
}

/// Where a thread on `repo` lives on `/repo`, when it is on.
fn thread_path(links: &Links, repo: &RepoAddress, thread: &Event) -> Option<String> {
    let section = if thread.kind.as_u16() == activity::ISSUE {
        "issues"
    } else {
        "pulls"
    };
    links
        .repo_path(repo)
        .map(|base| format!("{base}/{section}/{}", thread.id.to_hex()))
}

/// One row of recent activity: what kind, what about, when.
fn activity_row(links: &Links, event: &Event, targets: &HashMap<EventId, Event>) -> String {
    let repo = RepoAddress::tagged(event, "a")
        .into_iter()
        .chain(RepoAddress::tagged(event, "A"))
        .next();
    let on = repo
        .as_ref()
        .map(|repo| {
            format!(
                " <span class=\"muted\">in</span> <span class=\"mono\">{}</span>",
                escape(repo.d())
            )
        })
        .unwrap_or_default();
    let post_path = post::event_path(event);
    let (text, href) = match event.kind.as_u16() {
        activity::PATCH | activity::PULL_REQUEST | activity::ISSUE => (
            escape(&subject(event)),
            repo.as_ref()
                .and_then(|repo| thread_path(links, repo, event)),
        ),
        kind if activity::is_status(Kind::from(kind)) || kind == activity::PULL_UPDATE => {
            match target_id(event).and_then(|id| targets.get(&id)) {
                Some(target) => (
                    escape(&subject(target)),
                    repo.as_ref()
                        .and_then(|repo| thread_path(links, repo, target)),
                ),
                None => (escape(&content::first_line(&event.content, 80)), None),
            }
        }
        activity::COMMENT => (
            format!("“{}”", escape(&content::first_line(&event.content, 100))),
            Some(post_path.clone()),
        ),
        activity::CI_JOB | activity::CI_WORKFLOW => (
            escape(
                first_tag(event, "status")
                    .or_else(|| first_tag(event, "conclusion"))
                    .unwrap_or("result"),
            ),
            None,
        ),
        _ => (
            escape(&content::first_line(&event.content, 100)),
            Some(post_path.clone()),
        ),
    };
    let text = match href {
        Some(href) => format!("<a href=\"{}\">{text}</a>", escape(&href)),
        None => text,
    };
    format!(
        "<li class=\"activity-row\" data-kind=\"{}\" data-id=\"{}\"><span class=\"chip\">{}</span>\
         <span class=\"what\">{text}{on}</span><span class=\"muted small\">{}</span></li>",
        event.kind.as_u16(),
        event.id.to_hex(),
        activity::label(event),
        ago(event.created_at),
    )
}

/// The Posts tab: notes that start a conversation and articles, newest
/// first, drawn as the feed draws them.
async fn posts(pages: &Pages, links: &Links, key: &PublicKey) -> String {
    let store = &pages.store;
    let mut events = store
        .query(
            Filter::new()
                .author(*key)
                .kinds([Kind::from(NOTE), Kind::from(ARTICLE)])
                .limit(POSTS * 3),
        )
        .await;
    events.sort_by(|a, b| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
    let shown: Vec<Event> = events
        .into_iter()
        .filter(|event| event.kind.as_u16() != NOTE || !crate::is_reply(event))
        .take(POSTS)
        .collect();
    let mut repos: Vec<RepoAddress> = Vec::new();
    for event in &shown {
        for repo in post::repos_of(event) {
            if !repos.contains(&repo) {
                repos.push(repo);
            }
        }
    }
    let cards = load_cards(store, repos).await;
    let mut keys = vec![*key];
    for mentioned in shown
        .iter()
        .flat_map(|event| content::mentioned_keys(&event.content))
    {
        if !keys.contains(&mentioned) {
            keys.push(mentioned);
        }
    }
    let profiles = Profiles::load(store, &keys).await;
    let no_demos = HashMap::new();
    let view = View {
        links,
        names: profiles.names(),
        cards: &cards,
        demos: &no_demos,
        gateway: pages.gateway(),
    };
    let mut out = format!(
        "<p class=\"muted small\">Notes by {} that start a conversation (kind 1) and articles \
         (kind 30023), newest first. Replies are on the posts they answer.</p>",
        name(profiles.names(), key)
    );
    if shown.is_empty() {
        out.push_str("<div class=\"box\"><div class=\"empty\">No posts yet.</div></div>");
        return out;
    }
    out.push_str("<div class=\"feed\">");
    for event in &shown {
        out.push_str(&match (event.kind.as_u16(), view.ship(event)) {
            (ARTICLE, _) => post::article_card(&view, event),
            (_, Some(repo)) => post::ship_card(&view, event, &repo),
            _ => post::note_card(&view, event),
        });
    }
    out.push_str("</div>");
    out
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
    fn a_key_keeps_the_repos_it_owns_then_those_it_maintains() {
        let (me, other, stranger) = (Keys::generate(), Keys::generate(), Keys::generate());
        let me_hex = me.public_key().to_hex();
        let announcements = [
            signed(&me, ANNOUNCEMENT, 1, "", &[&["d", "old"]]),
            signed(&me, ANNOUNCEMENT, 3, "", &[&["d", "new"]]),
            signed(
                &other,
                ANNOUNCEMENT,
                2,
                "",
                &[&["d", "theirs"], &["maintainers", &me_hex]],
            ),
            // The owner's newer announcement dropped the key.
            signed(
                &other,
                ANNOUNCEMENT,
                1,
                "",
                &[&["d", "dropped"], &["maintainers", &me_hex]],
            ),
            signed(&other, ANNOUNCEMENT, 5, "", &[&["d", "dropped"]]),
            signed(&stranger, ANNOUNCEMENT, 4, "", &[&["d", "unrelated"]]),
        ];
        let kept = kept_repos(&me.public_key(), &announcements);
        let names: Vec<(&str, Role)> = kept.iter().map(|(repo, role)| (repo.d(), *role)).collect();
        assert_eq!(
            names,
            [
                ("new", Role::Owner),
                ("old", Role::Owner),
                ("theirs", Role::Maintainer)
            ]
        );
    }

    #[test]
    fn a_star_is_counted_once_and_only_a_plus_on_a_repo() {
        let (me, owner) = (Keys::generate(), Keys::generate());
        let a = format!("30617:{}:r", owner.public_key().to_hex());
        let b = format!("30617:{}:s", owner.public_key().to_hex());
        let reactions = [
            signed(&me, REACTION, 1, "+", &[&["a", &a], &["k", "30617"]]),
            signed(&me, REACTION, 2, "+", &[&["a", &a], &["k", "30617"]]),
            signed(&me, REACTION, 3, "-", &[&["a", &b], &["k", "30617"]]),
            signed(&me, REACTION, 4, "+", &[&["a", &b], &["k", "30617"]]),
            signed(&owner, REACTION, 5, "+", &[&["a", &a], &["k", "30617"]]),
        ];
        let found = starred(&me.public_key(), &reactions);
        assert_eq!(
            found,
            [
                RepoAddress::parse(&b).unwrap(),
                RepoAddress::parse(&a).unwrap()
            ]
        );
    }

    #[test]
    fn pins_keep_their_order_and_only_name_the_keys_own_repos() {
        let (me, other) = (Keys::generate(), Keys::generate());
        let mine = |d: &str| RepoAddress::new(me.public_key(), d).unwrap();
        let kept = vec![(mine("a"), Role::Owner), (mine("b"), Role::Owner)];
        let foreign = format!("30617:{}:x", other.public_key().to_hex());
        let list = signed(
            &me,
            PINNED_REPOS,
            1,
            "",
            &[
                &["a", &mine("b").coordinate()],
                &["a", &foreign],
                &["a", &mine("a").coordinate()],
            ],
        );
        assert_eq!(
            pinned(&me.public_key(), &[list], &kept),
            [mine("b"), mine("a")]
        );
        assert!(pinned(&me.public_key(), &[], &kept).is_empty());
    }

    #[test]
    fn followers_are_distinct_people_whose_list_names_the_key() {
        let (me, a, b) = (Keys::generate(), Keys::generate(), Keys::generate());
        let hex = me.public_key().to_hex();
        let lists = [
            signed(&a, FOLLOWS, 1, "", &[&["p", &hex]]),
            signed(&a, FOLLOWS, 2, "", &[&["p", &hex]]),
            signed(&b, FOLLOWS, 1, "", &[&["p", &a.public_key().to_hex()]]),
            signed(&me, FOLLOWS, 1, "", &[&["p", &hex]]),
        ];
        assert_eq!(followers(&me.public_key(), &lists), 1);
        let mine = [signed(
            &me,
            FOLLOWED_REPOS,
            1,
            "",
            &[&["a", "x"], &["a", "x"], &["a", "y"]],
        )];
        assert_eq!(list_size(&me.public_key(), &mine, FOLLOWED_REPOS, "a"), 2);
    }

    #[test]
    fn a_tab_is_its_path_segment() {
        for tab in Tab::ALL {
            if tab != Tab::Pinned {
                assert_eq!(Tab::from_slug(tab.slug()), Some(tab));
            }
        }
        assert_eq!(Tab::from_slug(""), None);
        assert_eq!(Tab::from_slug("agents"), None);
    }

    #[test]
    fn a_status_points_at_its_thread_by_root_first() {
        let keys = Keys::generate();
        let (root, other) = ("a".repeat(64), "b".repeat(64));
        let status = signed(
            &keys,
            activity::STATUS_APPLIED,
            1,
            "",
            &[&["e", &other, "", "reply"], &["e", &root, "", "root"]],
        );
        assert_eq!(target_id(&status), EventId::from_hex(&root).ok());
        let bare = signed(&keys, activity::PULL_UPDATE, 1, "", &[&["e", &other]]);
        assert_eq!(target_id(&bare), EventId::from_hex(&other).ok());
    }
}
