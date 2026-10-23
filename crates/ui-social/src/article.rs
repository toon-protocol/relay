//! The article reader (proposal 12, devlogs and series): one NIP-23 article
//! (kind 30023) at `/social/a/<naddr>`, or `/social/a/<npub>/<d>`.
//!
//! It shows the article's cover (`image`, `https` only, loaded by the
//! visitor's browser like any `/social` media), title, summary, when it was
//! first published (`published_at`) and how long it takes to read, its
//! author, and its Markdown through `ui_markdown`: a fixed set of tags,
//! raw HTML shown as text, links only to `http(s)`, `mailto`, in-page
//! anchors and `nostr:` URIs (which go to this app's pages). `nostr:`
//! mentions in the text are drawn as `/social` notes draw them, and a
//! paragraph that is only a mention of a code snippet embeds the snippet.
//!
//! Around it:
//!
//! - the repo it `a`-tags, as a repo card;
//! - a series nav: each NIP-51 curation set (kind 30004) by the author that
//!   holds the article, its parts in order ([`crate::series`]);
//! - a highlights rail: NIP-84 highlights (kind 9802) whose `a` is this
//!   article (or whose `e` is this version), grouped by passage and counted
//!   by person, the most-highlighted marked in the text
//!   ([`crate::highlights`]);
//! - reactions (kind 7 by `a` or `e`) and boosts (kind 16 by `a`) counted
//!   by person, and NIP-22 comments (kind 1111 whose root `A` is the
//!   article), listed below it.
//!
//! An article is addressable: an edit replaces it, so the reader shows the
//! newest version the store holds.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use nostr::event::{Event, EventId, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use nostr::nips::nip01::Coordinate;
use nostr::nips::nip19::{FromBech32, Nip19, Nip19Coordinate, ToBech32};
use nostr::nips::nip21::Nip21;
use nostr::parser::{NostrParser, Token};
use nostr::types::{RelayUrl, Timestamp};
use relay_ui::chrome::{
    Profiles, ago, escape, name, npub, percent_encode, person, person_avatar, plain_name,
};
use ui_markdown::{Node, Place};
use ui_nip34::card::plural;
use ui_nip34::repo::{RepoAddress, first_tag, newest, topics};

use crate::highlights::{self, Marker, Passage};
use crate::kinds::{ARTICLE, COMMENT, GENERIC_REPOST, HIGHLIGHT, REACTION, SNIPPET};
use crate::post::{self, View};
use crate::series::{self, Series};
use crate::{PREFIX, Pages, content, html, load_cards, not_found, note, parse_pubkey, snippet};

/// The most events one of the page's queries reads.
const LIMIT: usize = 2_000;
/// Words read in a minute, for the reading time.
const WORDS_PER_MINUTE: usize = 230;
/// The most passages the highlights rail lists.
const RAIL: usize = 20;
/// The most snippets an article embeds.
const EMBEDS: usize = 12;

/// The address of an article: its author and its `d`. An empty `d` is a
/// valid address (NIP-01), so it is allowed.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct ArticleAddress {
    author: PublicKey,
    d: String,
}

impl ArticleAddress {
    /// The article `d` by `author`.
    pub(crate) fn new(author: PublicKey, d: &str) -> Self {
        Self {
            author,
            d: d.to_owned(),
        }
    }

    /// The address of a kind 30023 event.
    pub(crate) fn of(event: &Event) -> Option<Self> {
        (event.kind.as_u16() == ARTICLE)
            .then(|| Self::new(event.pubkey, ui_nip34::repo::d_tag(event)))
    }

    /// The address an `a` tag value spells, `30023:<hex>:<d>`.
    pub(crate) fn parse(coordinate: &str) -> Option<Self> {
        let mut parts = coordinate.splitn(3, ':');
        let (kind, author, d) = (parts.next()?, parts.next()?, parts.next()?);
        (kind.parse::<u16>().ok()? == ARTICLE).then_some(())?;
        Some(Self::new(PublicKey::from_hex(author).ok()?, d))
    }

    /// The address a NIP-01 coordinate names, if it is an article's.
    pub(crate) fn from_coordinate(coordinate: &Coordinate) -> Option<Self> {
        (coordinate.kind.as_u16() == ARTICLE)
            .then(|| Self::new(coordinate.public_key, &coordinate.identifier))
    }

    /// The address an `naddr` spells, with or without `nostr:` before it,
    /// if it names an article.
    pub(crate) fn from_naddr(text: &str) -> Option<Self> {
        let text = text.strip_prefix("nostr:").unwrap_or(text);
        Self::from_coordinate(&Nip19Coordinate::from_bech32(text).ok()?.coordinate)
    }

    /// The author's key.
    pub(crate) fn author(&self) -> &PublicKey {
        &self.author
    }

    /// The `d` tag.
    pub(crate) fn d(&self) -> &str {
        &self.d
    }

    /// `30023:<author hex>:<d>`, as an `a` tag carries it.
    pub(crate) fn coordinate(&self) -> String {
        format!("{ARTICLE}:{}:{}", self.author.to_hex(), self.d)
    }

    /// The reader's page: `/social/a/<naddr>`, or `/social/a/<npub>/<d>`
    /// if the address cannot be an `naddr`.
    pub(crate) fn path(&self) -> String {
        let coordinate = Coordinate::new(Kind::from(ARTICLE), self.author).identifier(&self.d);
        match Nip19Coordinate::new(coordinate, Vec::<RelayUrl>::new()).to_bech32() {
            Ok(naddr) => format!("{PREFIX}/a/{naddr}"),
            Err(_) => format!(
                "{PREFIX}/a/{}/{}",
                npub(&self.author),
                percent_encode(&self.d)
            ),
        }
    }
}

/// The minutes `text` takes to read, at least one. A pure function.
pub(crate) fn reading_minutes(text: &str) -> usize {
    text.split_whitespace()
        .count()
        .div_ceil(WORDS_PER_MINUTE)
        .max(1)
}

/// Where a link in an article goes: the plain policy's links, and a
/// `nostr:` URI to this app's page for what it names. A pure function.
fn link(url: &str) -> Option<String> {
    if let Some(found) = ui_markdown::absolute_only(url) {
        return Some(found);
    }
    let bech32 = url.strip_prefix("nostr:")?;
    match Nip19::from_bech32(bech32).ok()? {
        Nip19::Pubkey(key) => Some(format!("{PREFIX}/p/{}", npub(&key))),
        Nip19::Profile(profile) => Some(format!("{PREFIX}/p/{}", npub(&profile.public_key))),
        Nip19::EventId(id) => Some(format!("{PREFIX}/e/{}", id.to_hex())),
        Nip19::Event(event) => Some(format!("{PREFIX}/e/{}", event.event_id.to_hex())),
        Nip19::Coordinate(coordinate) => {
            let coordinate = coordinate.coordinate;
            if let Some(article) = ArticleAddress::from_coordinate(&coordinate) {
                return Some(article.path());
            }
            RepoAddress::new(coordinate.public_key, &coordinate.identifier)
                .filter(|_| coordinate.kind.as_u16() == ui_nip34::repo::ANNOUNCEMENT)
                .map(|repo| format!("{PREFIX}/r/{}", repo.path_segments()))
        }
        _ => None,
    }
}

/// The ids of the events `text` mentions (`nostr:note`, `nostr:nevent`),
/// once each, at most `limit`.
fn mentioned_ids(text: &str, limit: usize) -> Vec<EventId> {
    let mut ids: Vec<EventId> = Vec::new();
    for token in NostrParser::new().parse(text) {
        let id = match token {
            Token::Nostr(Nip21::EventId(id)) => id,
            Token::Nostr(Nip21::Event(event)) => event.event_id,
            _ => continue,
        };
        if !ids.contains(&id) {
            ids.push(id);
            if ids.len() == limit {
                break;
            }
        }
    }
    ids
}

/// The event a paragraph names when a `nostr:` mention of it is all the
/// paragraph holds.
fn sole_mention(node: &Node) -> Option<EventId> {
    let Node::Element(element) = node else {
        return None;
    };
    match (element.tag, element.children.as_slice()) {
        ("p", [Node::Text(text)]) if text.trim().starts_with("nostr:") => {
            crate::parse_event_id(text.trim())
        }
        _ => None,
    }
}

/// One text node of the article: its `nostr:` mentions and links drawn
/// (outside links and code), and the passages `marker` finds marked.
fn prose(
    text: &str,
    place: Place,
    marker: &mut Marker,
    names: &HashMap<PublicKey, String>,
) -> String {
    let draw = |part: &str| {
        if place.in_link || place.in_code {
            escape(part)
        } else {
            content::render(part, &[], names).html
        }
    };
    if place.in_code {
        return escape(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    for (start, end, people) in marker.ranges(text) {
        out.push_str(&draw(&text[at..start]));
        out.push_str(&format!(
            "<mark class=\"hl\" title=\"{people} {} highlighted this\">{}</mark>",
            plural(people, "person", "people"),
            draw(&text[start..end])
        ));
        at = end;
    }
    out.push_str(&draw(&text[at..]));
    out
}

/// The article's body as HTML: its Markdown, with mentions, marks and the
/// snippets it embeds.
fn body_html(
    view: &View<'_>,
    article: &Event,
    marker: &mut Marker,
    snippets: &HashMap<EventId, Event>,
) -> String {
    let nodes = ui_markdown::parse(&article.content, link);
    let mut out = String::new();
    for node in &nodes {
        if let Some(embedded) = sole_mention(node).and_then(|id| snippets.get(&id)) {
            out.push_str(&snippet::card(view, embedded));
            continue;
        }
        out.push_str(&ui_markdown::to_html_with(
            std::slice::from_ref(node),
            &mut |text, place| prose(text, place, marker, view.names),
        ));
    }
    out
}

/// The newest version of the article at `address` the store holds.
async fn load(store: &relay_ui::ReadOnlyStore, address: &ArticleAddress) -> Option<Event> {
    let versions = store
        .query(
            Filter::new()
                .kind(Kind::from(ARTICLE))
                .author(address.author)
                .identifier(address.d.clone())
                .limit(50),
        )
        .await;
    newest(
        versions
            .iter()
            .filter(|event| ArticleAddress::of(event).as_ref() == Some(address)),
    )
    .cloned()
}

/// Whether `event` has a tag `name` whose value is `value`.
fn tagged(event: &Event, name: &str, value: &str) -> bool {
    event.tags.iter().map(|tag| tag.as_slice()).any(
        |tag| matches!(tag, [tag_name, tag_value, ..] if tag_name == name && tag_value == value),
    )
}

/// Events of `kind` that name the article by `a` (its address) or, when
/// `by_id`, by `e` (this version), each once.
async fn naming(
    store: &relay_ui::ReadOnlyStore,
    kind: u16,
    article: &Event,
    coordinate: &str,
    by_id: bool,
) -> Vec<Event> {
    let mut found: Vec<Event> = store
        .query(
            Filter::new()
                .kind(Kind::from(kind))
                .custom_tag(SingleLetterTag::LOWERCASE_A, coordinate.to_owned())
                .limit(LIMIT),
        )
        .await
        .into_iter()
        .filter(|event| tagged(event, "a", coordinate))
        .collect();
    if by_id {
        let id = article.id.to_hex();
        found.extend(
            store
                .query(
                    Filter::new()
                        .kind(Kind::from(kind))
                        .event(article.id)
                        .limit(LIMIT),
                )
                .await
                .into_iter()
                .filter(|event| tagged(event, "e", &id)),
        );
    }
    let mut seen = HashSet::new();
    found.retain(|event| seen.insert(event.id));
    found
}

pub(crate) async fn by_naddr(
    State(pages): State<Pages>,
    Path(target): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    match ArticleAddress::from_naddr(&target) {
        Some(address) => page(&pages, &address, query.as_deref()).await,
        None => not_found(&pages),
    }
}

pub(crate) async fn by_key(
    State(pages): State<Pages>,
    Path((author, d)): Path<(String, String)>,
    RawQuery(query): RawQuery,
) -> Response {
    match parse_pubkey(&author) {
        Some(author) => page(&pages, &ArticleAddress::new(author, &d), query.as_deref()).await,
        None => not_found(&pages),
    }
}

async fn page(pages: &Pages, address: &ArticleAddress, query: Option<&str>) -> Response {
    let store = &pages.store;
    let Some(article) = load(store, address).await else {
        return not_found(pages);
    };
    let coordinate = address.coordinate();
    let highlights = naming(store, HIGHLIGHT, &article, &coordinate, true).await;
    let reactions = naming(store, REACTION, &article, &coordinate, true).await;
    let boosts = naming(store, GENERIC_REPOST, &article, &coordinate, false).await;
    let mut comments: Vec<Event> = store
        .query(
            Filter::new()
                .kind(Kind::from(COMMENT))
                .custom_tag(SingleLetterTag::UPPERCASE_A, coordinate.clone())
                .limit(LIMIT),
        )
        .await
        .into_iter()
        .filter(|comment| tagged(comment, "A", &coordinate))
        .collect();
    comments.sort_by_key(|comment| (comment.created_at, comment.id));

    let sets = store
        .query(
            Filter::new()
                .kind(Kind::from(crate::kinds::CURATION_SET))
                .author(address.author)
                .custom_tag(SingleLetterTag::LOWERCASE_A, coordinate.clone())
                .limit(200),
        )
        .await;
    let series = series::series_of(&sets, address);
    let titles = part_titles(store, &series).await;

    let ids = mentioned_ids(&article.content, EMBEDS);
    let snippets: HashMap<EventId, Event> = if ids.is_empty() {
        HashMap::new()
    } else {
        store
            .query(Filter::new().ids(ids).kind(Kind::from(SNIPPET)))
            .await
            .into_iter()
            .map(|event| (event.id, event))
            .collect()
    };

    let repos = post::repos_of(&article);
    let cards = load_cards(store, repos.clone()).await;
    let passages = highlights::passages(&highlights);

    let mut keys: Vec<PublicKey> = vec![article.pubkey];
    let more = passages
        .iter()
        .flat_map(|passage| passage.people.iter().take(2).copied())
        .chain(comments.iter().map(|comment| comment.pubkey))
        .chain(snippets.values().map(|snippet| snippet.pubkey))
        .chain(content::mentioned_keys(&article.content))
        .chain(
            comments
                .iter()
                .flat_map(|comment| content::mentioned_keys(&comment.content)),
        )
        .chain(cards.values().map(|card| *card.address().owner()));
    for key in more {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let profiles = Profiles::load(store, &keys).await;
    let links = pages.links();
    let no_demos = HashMap::new();
    let view = View {
        links: &links,
        names: profiles.names(),
        bots: profiles.bots(),
        cards: &cards,
        demos: &no_demos,
        gateway: pages.gateway(),
    };

    let title = first_tag(&article, "title")
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| content::first_line(&article.content, 120));
    let mut marker = Marker::new(&passages);
    let around = Around {
        series: series.first(),
        snippets: &snippets,
        repos: &repos,
        reactions: &reactions,
        boosts: &boosts,
        comments: &comments,
    };
    let main = reader(&view, &article, &title, &mut marker, &around);
    let mut reported = crate::reports::Targets::default();
    reported.address(coordinate.clone(), "this article".to_owned());
    reported.event(article.id, "this article".to_owned());
    let mut body = crate::reports::page_banner(store, &reported, query, None).await;
    body.push_str(&format!(
        "<div class=\"reader{}\">",
        if series.is_empty() { " no-series" } else { "" }
    ));
    if !series.is_empty() {
        body.push_str(&series_nav(&series, &titles));
    }
    body.push_str(&main);
    body.push_str(&rail(&view, &passages));
    body.push_str("</div>");
    html(
        pages,
        crate::Nav::Other,
        StatusCode::OK,
        if title.is_empty() { "Article" } else { &title },
        None,
        &body,
    )
}

/// The titles of the parts of `series` this relay holds, by address.
async fn part_titles(
    store: &relay_ui::ReadOnlyStore,
    series: &[Series],
) -> HashMap<ArticleAddress, String> {
    let parts: Vec<&ArticleAddress> = series.iter().flat_map(|one| one.parts.iter()).collect();
    if parts.is_empty() {
        return HashMap::new();
    }
    let authors: HashSet<PublicKey> = parts.iter().map(|part| part.author).collect();
    let ds: HashSet<String> = parts.iter().map(|part| part.d.clone()).collect();
    let found = store
        .query(
            Filter::new()
                .kind(Kind::from(ARTICLE))
                .authors(authors)
                .identifiers(ds)
                .limit(LIMIT),
        )
        .await;
    let mut titles = HashMap::new();
    for part in parts {
        if titles.contains_key(part) {
            continue;
        }
        let version = newest(
            found
                .iter()
                .filter(|event| ArticleAddress::of(event).as_ref() == Some(part)),
        );
        if let Some(version) = version {
            let title = first_tag(version, "title")
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .map(str::to_owned)
                .unwrap_or_else(|| content::first_line(&version.content, 80));
            titles.insert(part.clone(), title);
        }
    }
    titles
}

/// The series nav: each series' parts in order, the one being read marked.
/// A part this relay does not hold is named by its `d`, unlinked.
fn series_nav(series: &[Series], titles: &HashMap<ArticleAddress, String>) -> String {
    let mut out = String::from("<nav class=\"series\" aria-label=\"Series\">");
    for one in series {
        out.push_str(&format!(
            "<h2 class=\"series-title\">Series &middot; {}</h2><ol class=\"parts\">",
            escape(&one.title)
        ));
        for (index, part) in one.parts.iter().enumerate() {
            let number = index + 1;
            let current = index == one.current;
            match titles.get(part) {
                Some(title) if current => out.push_str(&format!(
                    "<li><a class=\"part current\" href=\"{}\" aria-current=\"page\">\
                     <span class=\"mono\">{number}</span>{}</a></li>",
                    escape(&part.path()),
                    escape(title)
                )),
                Some(title) => out.push_str(&format!(
                    "<li><a class=\"part\" href=\"{}\"><span class=\"mono muted\">{number}</span>{}</a></li>",
                    escape(&part.path()),
                    escape(title)
                )),
                None => out.push_str(&format!(
                    "<li><span class=\"part missing\"><span class=\"mono muted\">{number}</span>\
                     <span class=\"muted\">{} (not on this relay)</span></span></li>",
                    escape(&part.d)
                )),
            }
        }
        out.push_str("</ol>");
        if let Some(description) = &one.description {
            out.push_str(&format!(
                "<p class=\"muted small\">{}</p>",
                escape(description)
            ));
        }
    }
    out.push_str(
        "<p class=\"muted small\">A series is a set of articles the author keeps in order \
         (a NIP-51 curation set, kind 30004).</p></nav>",
    );
    out
}

/// What the reader draws around the article's text.
struct Around<'a> {
    /// The first series the article is part of.
    series: Option<&'a Series>,
    /// The snippets its text mentions, by id.
    snippets: &'a HashMap<EventId, Event>,
    /// The repos it names.
    repos: &'a [RepoAddress],
    reactions: &'a [Event],
    boosts: &'a [Event],
    comments: &'a [Event],
}

/// The article itself.
fn reader(
    view: &View<'_>,
    article: &Event,
    title: &str,
    marker: &mut Marker,
    around: &Around<'_>,
) -> String {
    let repos = around.repos;
    let mut out = format!(
        "<article class=\"reader-main\" data-kind=\"30023\" data-id=\"{}\">\
         <p><a class=\"muted small\" href=\"{PREFIX}\">&larr; Back to the feed</a></p>",
        article.id.to_hex()
    );
    if let Some(cover) = crate::media::image_tag(article) {
        out.push_str(&cover.html("cover"));
    }
    let published = first_tag(article, "published_at")
        .and_then(|at| at.trim().parse::<u64>().ok())
        .map(Timestamp::from)
        .unwrap_or(article.created_at);
    let mut meta: Vec<String> = Vec::new();
    if let Some(series) = around.series {
        meta.push(format!(
            "Part {} of {}",
            series.current + 1,
            series.parts.len()
        ));
    }
    meta.push(format!("{} min read", reading_minutes(&article.content)));
    meta.push(format!("published {}", ago(published)));
    if article.created_at.as_secs() > published.as_secs() + 3_600 {
        meta.push(format!("updated {}", ago(article.created_at)));
    }
    out.push_str(&format!(
        "<p class=\"reader-meta muted\">{}</p><h1 class=\"article-title\">{}</h1>",
        meta.join(" &middot; "),
        escape(title)
    ));
    if let Some(summary) = first_tag(article, "summary").filter(|text| !text.trim().is_empty()) {
        out.push_str(&format!(
            "<p class=\"article-lead\">{}</p>",
            escape(summary.trim())
        ));
    }
    // The author, and the repo they write about.
    let repo = repos.iter().find(|repo| view.cards.contains_key(*repo));
    let about = repo
        .and_then(|repo| view.cards.get(repo))
        .map(|card| {
            let href = view
                .links
                .social_path(card.address())
                .unwrap_or_else(|| format!("{PREFIX}/r/{}", card.address().path_segments()));
            format!(
                "<span class=\"muted small\">{} <a href=\"{}\">{} / {}</a></span>",
                if card.is_maker(&article.pubkey) {
                    "devlog of"
                } else {
                    "writing about"
                },
                escape(&href),
                escape(&plain_name(card.profiles().names(), card.address().owner())),
                escape(card.name())
            )
        })
        .unwrap_or_default();
    let maker = repo.is_some_and(|repo| view.is_maker(repo, &article.pubkey));
    out.push_str(&format!(
        "<div class=\"article-by\">{}<div class=\"who\"><div class=\"line\">\
         {}{}{}</div>{about}</div></div>",
        person_avatar(view.links.apps(), view.names, &article.pubkey, false),
        person(view.links.apps(), view.names, &article.pubkey),
        relay_ui::chrome::bot_chip(view.bots, &article.pubkey),
        if maker {
            "<span class=\"chip chip-maker\">Maker</span>"
        } else {
            ""
        },
    ));
    out.push_str(&format!(
        "<div class=\"markdown article-body\">{}</div>",
        body_html(view, article, marker, around.snippets)
    ));
    let topics = topics(article);
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
    out.push_str(&post::cards_html(view, repos));

    // Counts, by person.
    let boosters: HashSet<PublicKey> = around.boosts.iter().map(|boost| boost.pubkey).collect();
    let commenters: HashSet<PublicKey> = around
        .comments
        .iter()
        .map(|comment| comment.pubkey)
        .collect();
    let mut chips = String::from("<div class=\"chips\" aria-label=\"Reactions\">");
    for (content, people) in note::tally(around.reactions).into_iter().take(6) {
        chips.push_str(&format!(
            "<span class=\"chip\" data-count=\"reactions\" title=\"{people} {}\">{} {people}</span>",
            plural(people, "person", "people"),
            escape(&content)
        ));
    }
    chips.push_str(&format!(
        "<span class=\"chip\" data-count=\"boosts\">{} {}</span>\
         <span class=\"chip\" data-count=\"comments\">{} {} from {} {}</span></div>",
        boosters.len(),
        plural(boosters.len(), "person boosted", "people boosted"),
        around.comments.len(),
        plural(around.comments.len(), "comment", "comments"),
        commenters.len(),
        plural(commenters.len(), "person", "people"),
    ));
    out.push_str(&chips);

    // Comments (NIP-22).
    out.push_str(&format!(
        "<section class=\"article-comments\" aria-label=\"Comments\"><h2>{} {}</h2>\
         <p class=\"muted small\">NIP-22 comments whose root is this article's address. They are \
         written in a client; each is a paid write.</p><div class=\"replies\">",
        around.comments.len(),
        plural(around.comments.len(), "comment", "comments"),
    ));
    for comment in around.comments {
        let maker = repo.is_some_and(|repo| view.is_maker(repo, &comment.pubkey))
            || comment.pubkey == article.pubkey;
        out.push_str(&note::reply_html(view, comment, maker));
    }
    out.push_str("</div></section></article>");
    out
}

/// The highlights rail: each passage, the people who highlighted it.
fn rail(view: &View<'_>, passages: &[Passage]) -> String {
    let people = highlights::people(passages);
    let mut out = String::from("<aside class=\"reader-rail\" aria-label=\"Highlights\">");
    if passages.is_empty() {
        out.push_str(
            "<h2>Highlights</h2><p class=\"muted small\">No one has highlighted this article yet.</p>",
        );
    } else {
        out.push_str(&format!(
            "<h2>{} {} highlighted by {} {}</h2>",
            passages.len(),
            plural(passages.len(), "passage", "passages"),
            people,
            plural(people, "person", "people"),
        ));
        let marker = Marker::new(passages);
        let marked: Vec<&str> = marker.passages().collect();
        for passage in passages.iter().take(RAIL) {
            let count = passage.people.len();
            let mut who: Vec<String> = passage
                .people
                .iter()
                .take(2)
                .map(|key| name(view.names, key))
                .collect();
            if count > 2 {
                who.push(format!("{} more", count - 2));
            }
            let who = match who.len() {
                0 | 1 => who.concat(),
                _ => {
                    let last = who.pop().unwrap_or_default();
                    format!("{} and {last}", who.join(", "))
                }
            };
            out.push_str(&format!(
                "<figure class=\"highlight{}\" data-people=\"{count}\"><blockquote>&ldquo;{}&rdquo;</blockquote>\
                 <figcaption class=\"muted small\">{who} &middot; {count} {}</figcaption></figure>",
                if marked.contains(&passage.text.as_str()) {
                    " marked"
                } else {
                    ""
                },
                escape(&passage.text),
                plural(count, "person", "people"),
            ));
        }
    }
    out.push_str(&format!(
        "<p class=\"muted small\">Highlights are passages readers saved (NIP-84, kind 9802, tagged \
         with this article). Marked in the text: the most-highlighted passages that {} or more people \
         chose, where the text is found word for word.</p></aside>",
        highlights::MARK_PEOPLE
    ));
    out
}

#[cfg(test)]
mod tests {
    use nostr::key::Keys;

    use super::*;

    #[test]
    fn an_article_address_is_read_in_each_spelling_and_links_to_the_reader() {
        let author = Keys::generate().public_key();
        let address = ArticleAddress::new(author, "tidepool-nsite");
        let coordinate = address.coordinate();
        assert_eq!(ArticleAddress::parse(&coordinate), Some(address.clone()));
        assert_eq!(
            ArticleAddress::parse(&format!("30617:{}:x", author.to_hex())),
            None
        );
        let path = address.path();
        let naddr = path.strip_prefix("/social/a/").unwrap();
        assert!(naddr.starts_with("naddr1"));
        assert_eq!(ArticleAddress::from_naddr(naddr), Some(address.clone()));
        assert_eq!(
            ArticleAddress::from_naddr(&format!("nostr:{naddr}")),
            Some(address)
        );
        let empty = ArticleAddress::new(author, "");
        assert_eq!(ArticleAddress::parse(&empty.coordinate()), Some(empty));
    }

    #[test]
    fn reading_time_rounds_up_and_is_never_zero() {
        assert_eq!(reading_minutes(""), 1);
        assert_eq!(reading_minutes(&"word ".repeat(230)), 1);
        assert_eq!(reading_minutes(&"word ".repeat(231)), 2);
    }

    #[test]
    fn a_nostr_link_goes_to_this_apps_page_and_others_follow_the_plain_policy() {
        let key = Keys::generate().public_key();
        assert_eq!(
            link(&format!("nostr:{}", npub(&key))),
            Some(format!("/social/p/{}", npub(&key)))
        );
        let article = ArticleAddress::new(key, "post");
        let naddr = article.path().trim_start_matches("/social/a/").to_owned();
        assert_eq!(link(&format!("nostr:{naddr}")), Some(article.path()));
        assert_eq!(link("https://x.test"), Some("https://x.test".to_owned()));
        assert_eq!(link("javascript:alert(1)"), None);
        assert_eq!(link("nostr:garbage"), None);
    }

    #[test]
    fn prose_marks_passages_and_draws_mentions_but_not_in_code_or_links() {
        let (a, b) = (Keys::generate(), Keys::generate());
        let passages = vec![Passage {
            text: "cache them for a year".to_owned(),
            people: vec![a.public_key(), b.public_key()],
        }];
        let names = HashMap::new();
        let key = Keys::generate().public_key();
        let mut marker = Marker::new(&passages);
        let code = prose(
            "cache them for a year <b>",
            Place {
                in_link: false,
                in_code: true,
            },
            &mut marker,
            &names,
        );
        assert_eq!(code, "cache them for a year &lt;b&gt;");
        let text = format!("So cache them for a year, says nostr:{} <b>", npub(&key));
        let drawn = prose(&text, Place::default(), &mut marker, &names);
        assert!(
            drawn.contains(
                "<mark class=\"hl\" title=\"2 people highlighted this\">cache them for a year</mark>"
            ),
            "{drawn}"
        );
        assert!(drawn.contains("href=\"/social/p/"), "{drawn}");
        assert!(drawn.contains("&lt;b&gt;"), "{drawn}");
        let linked = prose(
            &format!("nostr:{}", npub(&key)),
            Place {
                in_link: true,
                in_code: false,
            },
            &mut marker,
            &names,
        );
        assert!(!linked.contains("<a"), "no link inside a link: {linked}");
    }

    #[test]
    fn a_paragraph_that_is_only_a_mention_names_its_event() {
        let id = EventId::from_hex(&"ab".repeat(32)).unwrap();
        let note = id.to_bech32().unwrap();
        let nodes = ui_markdown::parse(
            &format!("nostr:{note}\n\nSee nostr:{note} here\n"),
            ui_markdown::absolute_only,
        );
        assert_eq!(sole_mention(&nodes[0]), Some(id));
        assert_eq!(sole_mention(&nodes[1]), None);
        assert_eq!(
            mentioned_ids(&format!("nostr:{note} and nostr:{note}"), 5),
            vec![id]
        );
    }
}
