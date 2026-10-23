//! A repo's social page, `/social/r/<npub>/<d>` (proposal 2 of
//! `docs/social-for-builders-ux.md`): the repo as a profile. One route per
//! tab, so the page needs no script:
//!
//! | Tab       | Path        | What it lists                                                  |
//! | --------- | ----------- | -------------------------------------------------------------- |
//! | Maker     | (none)      | posts about the repo by its owner and maintainers (ADR 0002)  |
//! | Talk      | `/talk`     | everyone else's notes, threads, comments (`A`) and highlights |
//! | Stars     | `/stars`    | each key that starred it (kind 7 `+`, `a`, `k=30617`)          |
//! | Followers | `/followers`| each key whose kind 10018 names it                             |
//! | Media     | `/media`    | pictures and videos (kinds 20, 21, 22) that `a`-tag it         |
//! | Snippets  | `/snippets` | code snippets (kind 1337) that `a`-tag it or name it in `repo` |
//!
//! Counts are of people where people are what is counted (stars, followers,
//! talk), and the store has already applied every NIP-09 deletion.

use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use nostr::event::{Event, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use nostr::types::Timestamp;
use relay_ui::chrome::{self, Profiles, ago, avatar, escape, icon, name, npub, plain_name, svg};
use ui_nip34::card::{self, RepoCard, clone_urls, plural};
use ui_nip34::repo::{RepoAddress, first_tag, tag_values, topics};

use crate::kinds::{
    APP_HANDLER, ARTICLE, COMMENT, GENERIC_REPOST, HIGHLIGHT, NOTE, PICTURE, SHORT_VIDEO, SNIPPET,
    THREAD, VIDEO,
};
use crate::media;
use crate::post::{self, View};
use crate::ship::{self, Site};
use crate::{PREFIX, Pages, html, not_found, parse_pubkey};

/// The kinds that name a repo in an `a` tag and appear on its page.
const TAGGED_KINDS: [u16; 11] = [
    NOTE,
    THREAD,
    GENERIC_REPOST,
    PICTURE,
    VIDEO,
    SHORT_VIDEO,
    SNIPPET,
    HIGHLIGHT,
    ARTICLE,
    APP_HANDLER,
    COMMENT,
];
/// The most events one of the page's queries reads.
const LIMIT: usize = 1_000;
/// The most recent snippets scanned for a `repo` tag naming the repo.
const SNIPPET_SCAN: usize = 500;

/// A tab of the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Maker,
    Talk,
    Stars,
    Followers,
    Media,
    Snippets,
}

impl Tab {
    const ALL: [Tab; 6] = [
        Tab::Maker,
        Tab::Talk,
        Tab::Stars,
        Tab::Followers,
        Tab::Media,
        Tab::Snippets,
    ];

    /// The tab a path segment names; the Maker tab has none.
    fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|tab| *tab != Tab::Maker && tab.slug() == slug)
    }

    fn slug(self) -> &'static str {
        match self {
            Self::Maker => "",
            Self::Talk => "talk",
            Self::Stars => "stars",
            Self::Followers => "followers",
            Self::Media => "media",
            Self::Snippets => "snippets",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Maker => "Maker",
            Self::Talk => "Talk",
            Self::Stars => "Stars",
            Self::Followers => "Followers",
            Self::Media => "Media",
            Self::Snippets => "Snippets",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Maker => icon::SHIP,
            Self::Talk => icon::COMMENT,
            Self::Stars => icon::STAR,
            Self::Followers => icon::EYE,
            Self::Media => icon::IMAGE,
            Self::Snippets => icon::CODE,
        }
    }
}

/// What the page lists, split by tab.
#[derive(Debug, Default)]
pub(crate) struct Sections {
    maker: Vec<Event>,
    talk: Vec<Event>,
    media: Vec<Event>,
    snippets: Vec<Event>,
}

impl Sections {
    /// The distinct authors of the Talk tab.
    fn talkers(&self) -> usize {
        self.talk
            .iter()
            .map(|event| event.pubkey)
            .collect::<HashSet<_>>()
            .len()
    }
}

/// `events` about a repo, split: pictures and videos to Media, snippets to
/// Snippets, the maker's notes, threads, comments, highlights, articles and
/// app events to Maker, everyone else's notes, threads, comments and
/// highlights to Talk. Boosts are counted nowhere. Each list is newest
/// first, each event once. A pure function.
pub(crate) fn sections(events: Vec<Event>, is_maker: impl Fn(&PublicKey) -> bool) -> Sections {
    let mut events = events;
    events.sort_by(|a, b| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
    events.dedup_by_key(|event| event.id);
    let mut sections = Sections::default();
    for event in events {
        match event.kind.as_u16() {
            PICTURE | VIDEO | SHORT_VIDEO => sections.media.push(event),
            SNIPPET => sections.snippets.push(event),
            NOTE | THREAD | COMMENT | HIGHLIGHT | ARTICLE | APP_HANDLER
                if is_maker(&event.pubkey) =>
            {
                sections.maker.push(event);
            }
            NOTE | THREAD | COMMENT | HIGHLIGHT => sections.talk.push(event),
            _ => {}
        }
    }
    sections
}

/// Whether a snippet's `repo` tag names `repo`: its address, an `naddr` of
/// it, or its `nostr://` URL. A pure function.
fn snippet_names(snippet: &Event, repo: &RepoAddress, nip05: Option<&str>) -> bool {
    tag_values(snippet, "repo").first().is_some_and(|value| {
        RepoAddress::parse(value).as_ref() == Some(repo)
            || RepoAddress::from_naddr(value).as_ref() == Some(repo)
            || ship::source_names(value, repo, nip05)
    })
}

/// `/social/r/<naddr>`: a redirect to the repo's page.
pub(crate) async fn redirect(State(pages): State<Pages>, Path(target): Path<String>) -> Response {
    match RepoAddress::from_naddr(&target) {
        Some(repo) => (
            StatusCode::MOVED_PERMANENTLY,
            [(
                header::LOCATION,
                format!("{PREFIX}/r/{}", repo.path_segments()),
            )],
        )
            .into_response(),
        None => not_found(&pages),
    }
}

pub(crate) async fn maker(
    State(pages): State<Pages>,
    Path((owner, d)): Path<(String, String)>,
) -> Response {
    page(&pages, &owner, &d, Tab::Maker).await
}

pub(crate) async fn tab(
    State(pages): State<Pages>,
    Path((owner, d, tab)): Path<(String, String, String)>,
) -> Response {
    match Tab::from_slug(&tab) {
        Some(tab) => page(&pages, &owner, &d, tab).await,
        None => not_found(&pages),
    }
}

async fn page(pages: &Pages, owner: &str, d: &str, tab: Tab) -> Response {
    let Some(repo) = parse_pubkey(owner).and_then(|owner| RepoAddress::new(owner, d)) else {
        return not_found(pages);
    };
    let store = &pages.store;
    let Some(card) = RepoCard::load(store, &repo).await else {
        return not_found(pages);
    };
    let coordinate = repo.coordinate();
    let mut events: Vec<Event> = store
        .query(
            Filter::new()
                .kinds(TAGGED_KINDS.map(Kind::from))
                .custom_tag(SingleLetterTag::LOWERCASE_A, coordinate.clone())
                .limit(LIMIT),
        )
        .await
        .into_iter()
        .filter(|event| RepoAddress::tagged(event, "a").contains(&repo))
        .collect();
    events.extend(
        store
            .query(
                Filter::new()
                    .kind(Kind::from(COMMENT))
                    .custom_tag(SingleLetterTag::UPPERCASE_A, coordinate.clone())
                    .limit(LIMIT),
            )
            .await
            .into_iter()
            .filter(|event| RepoAddress::tagged(event, "A").contains(&repo)),
    );
    let nip05 = card.profiles().nip05(repo.owner());
    events.extend(
        store
            .query(Filter::new().kind(Kind::from(SNIPPET)).limit(SNIPPET_SCAN))
            .await
            .into_iter()
            .filter(|event| snippet_names(event, &repo, nip05)),
    );
    let sections = sections(events, |key| card.is_maker(key));
    let stars = card::stargazers(store, &repo).await;
    let followers = card::followers(store, &repo).await;

    let mut keys: Vec<PublicKey> = card.authorized().to_vec();
    let listed: Vec<&Event> = match tab {
        Tab::Maker => sections.maker.iter().collect(),
        Tab::Talk => sections.talk.iter().collect(),
        Tab::Media => sections.media.iter().collect(),
        Tab::Snippets => sections.snippets.iter().collect(),
        Tab::Stars | Tab::Followers => Vec::new(),
    };
    let people = match tab {
        Tab::Stars => stars.as_slice(),
        Tab::Followers => followers.as_slice(),
        _ => &[],
    };
    for key in listed
        .iter()
        .map(|event| event.pubkey)
        .chain(people.iter().map(|(key, _)| *key))
    {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let profiles = Profiles::load(store, &keys).await;
    let demo = match pages.gateway() {
        Some(_) => ship::demo(store, &card).await,
        None => None,
    };
    let links = pages.links();
    let cards = std::collections::HashMap::from([(repo.clone(), card.clone())]);
    let demos: std::collections::HashMap<RepoAddress, Site> = demo
        .iter()
        .map(|site| (repo.clone(), site.clone()))
        .collect();
    let view = View {
        links: &links,
        names: profiles.names(),
        cards: &cards,
        demos: &demos,
        gateway: pages.gateway(),
    };

    let counts = |tab: Tab| match tab {
        Tab::Maker => sections.maker.len(),
        Tab::Talk => sections.talkers(),
        Tab::Stars => stars.len(),
        Tab::Followers => followers.len(),
        Tab::Media => sections.media.len(),
        Tab::Snippets => sections.snippets.len(),
    };
    let mut body = header(&view, &card, stars.len(), followers.len());
    let base = links
        .social_path(&repo)
        .unwrap_or_else(|| format!("{PREFIX}/r/{}", repo.path_segments()));
    let mut tabs: Vec<chrome::Tab> = Vec::new();
    if let Some(code) = links.repo_path(&repo) {
        tabs.push(chrome::Tab::new("Code", icon::CODE, code));
    }
    for each in Tab::ALL {
        let href = if each == Tab::Maker {
            base.clone()
        } else {
            format!("{base}/{}", each.slug())
        };
        tabs.push(
            chrome::Tab::new(each.label(), each.icon(), href)
                .current(each == tab)
                .count(counts(each)),
        );
    }
    body.push_str(&chrome::tabs("Repository", &tabs));
    body.push_str("<div class=\"split\"><div class=\"main\">");
    match tab {
        Tab::Maker => body.push_str(&posts(
            &view,
            &listed,
            true,
            "Only the owner and the maintainers the owner named: launches, devlogs and notes.",
            "The makers have not posted about this repo yet.",
        )),
        Tab::Talk => body.push_str(&posts(
            &view,
            &listed,
            false,
            &format!(
                "Everyone else: notes that tag this repo, comments on it (NIP-22), threads and \
                 highlights. {} {}, {} {}.",
                sections.talkers(),
                plural(sections.talkers(), "person", "people"),
                listed.len(),
                plural(listed.len(), "post", "posts"),
            ),
            "Nobody has talked about this repo yet.",
        )),
        Tab::Stars => body.push_str(&people_grid(
            &view,
            people,
            &format!(
                "{} {} starred this repo: a <code>+</code> reaction on its address, each a paid write.",
                people.len(),
                plural(people.len(), "person", "people")
            ),
            "starred",
            "No one has starred this repo yet.",
        )),
        Tab::Followers => body.push_str(&people_grid(
            &view,
            people,
            &format!(
                "{} {} this repo: it is in their git repositories list (kind 10018).",
                people.len(),
                plural(people.len(), "person follows", "people follow")
            ),
            "listed it",
            "No one follows this repo yet.",
        )),
        Tab::Media => body.push_str(&media_grid(&view, &listed)),
        Tab::Snippets => body.push_str(&snippets(&view, &listed)),
    }
    body.push_str("</div>");
    body.push_str(&sidebar(&view, &card));
    body.push_str("</div>");
    html(
        pages,
        StatusCode::OK,
        &format!("{} · {}", card.name(), tab.label()),
        None,
        &body,
    )
}

/// The repo's header: owner / name, the star and follow counts, the live
/// demo, its description and topics.
fn header(view: &View<'_>, card: &RepoCard, stars: usize, followers: usize) -> String {
    let repo = card.address();
    let base = view
        .links
        .social_path(repo)
        .unwrap_or_else(|| format!("{PREFIX}/r/{}", repo.path_segments()));
    let mut actions = format!(
        "<span class=\"actions\"><a class=\"btn\" href=\"{0}/stars\">{1}Stars<span class=\"count\">{stars}</span></a>\
         <a class=\"btn\" href=\"{0}/followers\">{2}Followers<span class=\"count\">{followers}</span></a>",
        escape(&base),
        svg(icon::STAR, "icon-sm"),
        svg(icon::EYE, "icon-sm"),
    );
    if let Some(url) = view.demo_url(repo) {
        actions.push_str(&format!(
            "<a class=\"btn btn-success\" href=\"{}\" rel=\"noopener noreferrer\" data-demo>{}Live demo</a>",
            escape(&url),
            svg(icon::PLAY, "icon-sm")
        ));
    }
    actions.push_str("</span>");
    let mut out = chrome::repo_title(
        &name(card.profiles().names(), repo.owner()),
        card.name(),
        &base,
        &actions,
    );
    if let Some(description) = card.description() {
        out.push_str(&format!(
            "<p class=\"repo-desc\">{}</p>",
            escape(description)
        ));
    }
    let topics = topics(card.announcement());
    if !topics.is_empty() {
        out.push_str("<div class=\"chips\">");
        for topic in topics {
            out.push_str(&view.links.topic_chip(topic));
        }
        out.push_str("</div>");
    }
    out
}

/// A tab's posts, under a line that says what the tab holds.
fn posts(view: &View<'_>, events: &[&Event], maker: bool, note: &str, empty: &str) -> String {
    let mut out = format!("<p class=\"muted small\">{note}</p>");
    if events.is_empty() {
        out.push_str(&format!(
            "<div class=\"box\"><div class=\"empty\">{empty}</div></div>"
        ));
        return out;
    }
    out.push_str("<div class=\"feed\">");
    for event in events {
        out.push_str(&post::repo_post(view, event, maker));
    }
    out.push_str("</div>");
    out
}

/// People, each linking to their following feed.
fn people_grid(
    view: &View<'_>,
    people: &[(PublicKey, Timestamp)],
    note: &str,
    verb: &str,
    empty: &str,
) -> String {
    let mut out = format!("<p class=\"muted small\">{note}</p>");
    if people.is_empty() {
        out.push_str(&format!(
            "<div class=\"box\"><div class=\"empty\">{empty}</div></div>"
        ));
        return out;
    }
    out.push_str("<div class=\"people\">");
    for (key, at) in people {
        out.push_str(&format!(
            "<a class=\"person\" href=\"{PREFIX}/following/{}\">{}<span class=\"who\">{}\
             <span class=\"muted small\">{verb} {}</span></span></a>",
            npub(key),
            avatar(&plain_name(view.names, key), false),
            name(view.names, key),
            ago(*at),
        ));
    }
    out.push_str("</div>");
    out
}

/// Pictures and videos, each with who posted it.
fn media_grid(view: &View<'_>, events: &[&Event]) -> String {
    let mut figures = String::new();
    for event in events {
        for item in media::imeta(event) {
            figures.push_str(&format!(
                "<figure>{}<figcaption>{} &middot; <a class=\"muted\" href=\"{}\">{}</a></figcaption></figure>",
                item.html(""),
                name(view.names, &event.pubkey),
                escape(&post::event_path(event)),
                first_tag(event, "title")
                    .map(escape)
                    .unwrap_or_else(|| crate::kinds::label(event.kind.as_u16()).to_owned()),
            ));
        }
    }
    let mut out = String::from(
        "<p class=\"muted small\">Pictures and videos (kinds 20, 21 and 22) that tag this repo. \
         Your browser loads them from where their posts say; this relay does not.</p>",
    );
    if figures.is_empty() {
        out.push_str("<div class=\"box\"><div class=\"empty\">No media yet.</div></div>");
    } else {
        out.push_str(&format!("<div class=\"media-grid\">{figures}</div>"));
    }
    out
}

/// Code snippets (NIP-C0), as text.
fn snippets(view: &View<'_>, events: &[&Event]) -> String {
    let mut out = String::from(
        "<p class=\"muted small\">Code snippets (kind 1337) that tag this repo or name it in their \
         <code>repo</code> tag.</p>",
    );
    if events.is_empty() {
        out.push_str("<div class=\"box\"><div class=\"empty\">No snippets yet.</div></div>");
        return out;
    }
    for event in events {
        let language = first_tag(event, "l")
            .map(|language| format!("<span class=\"chip\">{}</span>", escape(language)))
            .unwrap_or_default();
        out.push_str(&format!(
            "<article class=\"box snippet\" data-id=\"{}\"><div class=\"box-header\">\
             <span class=\"strong\">{}</span>{language}<span class=\"muted small\">by {} &middot; {}</span></div>\
             <pre><code>{}</code></pre></article>",
            event.id.to_hex(),
            escape(first_tag(event, "name").unwrap_or("snippet")),
            name(view.names, &event.pubkey),
            ago(event.created_at),
            escape(&event.content),
        ));
    }
    out
}

/// The sidebar: what the repo is, where to clone it, who keeps it, and how
/// to star or follow it from a client.
fn sidebar(view: &View<'_>, card: &RepoCard) -> String {
    let repo = card.address();
    let mut out = String::from("<aside class=\"sidebar\"><div><h2>About</h2>");
    match card.description() {
        Some(description) => out.push_str(&format!("<p>{}</p>", escape(description))),
        None => out.push_str("<p class=\"muted\"><em>No description provided.</em></p>"),
    }
    let web: Vec<&String> = tag_values(card.announcement(), "web")
        .iter()
        .filter(|url| url.starts_with("https://") || url.starts_with("http://"))
        .collect();
    if !web.is_empty() {
        out.push_str("<ul class=\"facts\">");
        for url in web {
            out.push_str(&format!(
                "<li>{}<a class=\"truncate\" href=\"{1}\" rel=\"nofollow noopener noreferrer\">{1}</a></li>",
                svg(icon::LINK, ""),
                escape(url)
            ));
        }
        out.push_str("</ul>");
    }
    if let Some((at, branch)) = card.last_move() {
        out.push_str(&format!(
            "<p class=\"muted small\">{}{} moved {}</p>",
            svg(icon::BRANCH, "icon-sm"),
            escape(branch),
            ago(at)
        ));
    }
    out.push_str("</div><div class=\"side-card\"><h2>Clone</h2>");
    for (url, claimed) in clone_urls(
        repo,
        view.links.relay_host(),
        card.profiles().nip05(repo.owner()),
    ) {
        out.push_str(&format!(
            "<code class=\"copy-field\">git clone {}</code>",
            escape(&url)
        ));
        if claimed {
            out.push_str("<p class=\"muted small\">The NIP-05 name is the owner's claim; this relay has not checked it.</p>");
        }
    }
    out.push_str("</div><div><h3>Maintainers</h3><ul>");
    for key in card.authorized() {
        out.push_str(&format!(
            "<li>{} {}</li>",
            avatar(&plain_name(card.profiles().names(), key), true),
            name(card.profiles().names(), key)
        ));
    }
    out.push_str(&format!(
        "</ul></div><div class=\"side-card\"><h2>Star or follow</h2><p class=\"muted small\">\
         This page writes nothing. From a Nostr client, a star is a kind 7 with content <code>+</code> \
         and the tags <code>a</code> <code>{0}</code> and <code>k</code> <code>30617</code>; a follow \
         adds <code>a</code> <code>{0}</code> to your kind 10018 list. Each is one paid write.</p></div></aside>",
        escape(&repo.coordinate())
    ));
    out
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;

    use super::*;

    fn signed(keys: &Keys, kind: u16, at: u64, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .custom_created_at(Timestamp::from(at))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn the_maker_is_kept_apart_from_the_crowd() {
        let maker = Keys::generate();
        let fan = Keys::generate();
        let events = vec![
            signed(&maker, NOTE, 1, &[]),
            signed(&fan, NOTE, 2, &[]),
            signed(&fan, COMMENT, 3, &[]),
            signed(&fan, NOTE, 4, &[]),
            signed(&maker, ARTICLE, 5, &[]),
            signed(&fan, ARTICLE, 6, &[]),
            signed(&fan, PICTURE, 7, &[]),
            signed(&maker, SNIPPET, 8, &[]),
            signed(&fan, GENERIC_REPOST, 9, &[]),
        ];
        let split = sections(events, |key| *key == maker.public_key());
        assert_eq!(split.maker.len(), 2);
        assert_eq!(split.maker[0].kind, Kind::from(ARTICLE), "newest first");
        assert_eq!(split.talk.len(), 3);
        assert_eq!(split.talkers(), 1, "one person, three posts");
        assert_eq!(split.media.len(), 1);
        assert_eq!(split.snippets.len(), 1);
    }

    #[test]
    fn a_snippet_names_its_repo_in_any_spelling() {
        let owner = Keys::generate();
        let repo = RepoAddress::new(owner.public_key(), "r").unwrap();
        let tagged = |value: &str| signed(&owner, SNIPPET, 1, &[&["repo", value]]);
        assert!(snippet_names(&tagged(&repo.coordinate()), &repo, None));
        assert!(snippet_names(
            &tagged(&format!(
                "nostr://{}/relay.test/r",
                npub(&owner.public_key())
            )),
            &repo,
            None
        ));
        assert!(!snippet_names(
            &tagged("https://github.com/x/r"),
            &repo,
            None
        ));
    }

    #[test]
    fn a_tab_is_its_path_segment() {
        for tab in Tab::ALL {
            if tab != Tab::Maker {
                assert_eq!(Tab::from_slug(tab.slug()), Some(tab));
            }
        }
        assert_eq!(Tab::from_slug(""), None);
        assert_eq!(Tab::from_slug("forks"), None);
    }
}
