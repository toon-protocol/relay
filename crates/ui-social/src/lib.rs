//! The social app UI, mounted at `/social`: the social NIPs for builders,
//! with repositories as first-class citizens (`docs/apps/social.md`,
//! `docs/social-for-builders-ux.md` §7).
//!
//! Server-rendered pages over this relay's store, with no script and no
//! login, read through a [`ReadOnlyStore`] and never writing (ADR 0001):
//!
//! - `/social`: everyone's notes, repo boosts, ship posts, articles, merges
//!   and new repos ([`feed`]), and `/social/following/<npub>` for the people
//!   and repos one key follows (kinds 3, 10017 and 10018);
//! - `/social/r/<npub>/<d>`: a repo's social page, with Maker, Talk, Stars,
//!   Followers, Forks, Media and Snippets tabs ([`repo_page`]; Forks is
//!   [`lineage`]), a live demo ([`demo`]) and "Open in…" apps ([`handlers`]);
//! - `/social/e/<id>`: one post, drawn as a launch when it is a ship post
//!   ([`note`], [`ship`]);
//! - `/social/trending`: repos ranked by distinct people per window, by a
//!   formula the page prints ([`trending`]);
//! - `/social/t` and `/social/t/<topic>`: topics, from repos' `t` tags and
//!   NIP-32 labels ([`topics`]);
//! - `/social/p/<npub>`: a builder's profile, with Pinned, Repositories,
//!   Starred and Posts tabs and a year of activity ([`profile`]);
//! - `/social/a/<naddr>`: an article (NIP-23) with its series, highlights
//!   and comments ([`article`], [`series`], [`highlights`]);
//! - `/social/s/<id>`: a code snippet (NIP-C0) ([`snippet`]);
//! - `/social/events` and `/social/ev/<npub>/<d>`: NIP-52 calendar events,
//!   their RSVPs, badges and submissions gallery ([`events`]);
//! - `/social/g/<id>`: a NIP-29 room, read-only, when the relay keeps
//!   groups ([`rooms`]).
//!
//! A mention of a repo anywhere unfurls into `ui_nip34`'s repo card, and the
//! page shell is `relay_ui::chrome`'s, so `/repo` and `/social` read as one
//! site. Media a post names is loaded by the visitor's browser, never by the
//! relay ([`media`]).

mod article;
mod badges;
mod calendar;
mod content;
mod demo;
mod events;
mod feed;
mod handlers;
mod highlights;
mod kinds;
mod lineage;
mod live;
mod media;
mod note;
mod post;
mod profile;
mod repo_page;
mod rooms;
mod series;
mod ship;
mod snippet;
mod topics;
mod trending;

use std::collections::HashMap;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::header::{CONTENT_SECURITY_POLICY, REFERRER_POLICY};
use axum::http::{HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use nostr::event::{Event, EventId};
use nostr::filter::Filter;
use nostr::key::PublicKey;
use nostr::nips::nip19::{FromBech32, Nip19};
use relay_ui::chrome::{self, App, Apps, escape, npub};
use relay_ui::{AppUi, ReadOnlyStore};
use ui_nip34::card::{Links, RepoCard};
use ui_nip34::repo::RepoAddress;

/// The mount prefix.
pub const PREFIX: &str = "/social";

/// What a `/social` page may load: its own stylesheet, and images and video
/// over `https` from anywhere, which the visitor's browser fetches for the
/// media a post names (the relay never does). No script at all, and a form
/// may only submit back to this origin.
pub const CONTENT_SECURITY_POLICY_VALUE: &str = "default-src 'self'; script-src 'none'; \
     img-src 'self' https:; media-src 'self' https:; form-action 'self'; base-uri 'none'";

/// The most repo cards one page loads: each is a handful of queries.
const CARD_LIMIT: usize = 24;

/// What the operator told `/social`. The relay writes these into pages and
/// never contacts them.
#[derive(Debug, Clone, Default)]
pub struct Settings {
    relay_host: Option<String>,
    nsite_gateway: Option<String>,
    group_signer: Option<PublicKey>,
}

impl Settings {
    /// `relay_host` is the host visitors' clients reach this relay at (the
    /// relay hint of a `nostr://` clone URL); `nsite_gateway` is the `https`
    /// URL of the gateway a live demo is linked through.
    pub fn new(relay_host: Option<String>, nsite_gateway: Option<String>) -> Self {
        Self {
            relay_host,
            nsite_gateway,
            group_signer: None,
        }
    }

    /// The relay keeps NIP-29 groups and signs their metadata with
    /// `signer`: `/social` shows their rooms. The store it is handed is
    /// already read as a visitor who has not signed in would, without what
    /// private and closed groups hold.
    pub fn with_group_signer(mut self, signer: PublicKey) -> Self {
        self.group_signer = Some(signer);
        self
    }
}

/// What the pages read.
#[derive(Clone)]
struct Pages {
    store: ReadOnlyStore,
    settings: Settings,
    apps: Apps,
    /// This relay's NIP-29 groups, when it keeps them.
    groups: Option<rooms::Groups>,
}

impl Pages {
    /// What a repo card links to.
    fn links(&self) -> Links {
        Links::new(self.apps.clone(), self.settings.relay_host.clone())
    }

    fn gateway(&self) -> Option<&str> {
        self.settings.nsite_gateway.as_deref()
    }
}

/// The app UI over `store`, with the operator's `settings` and the `apps`
/// the relay serves (for the switcher and the links to `/repo`).
pub fn app(store: ReadOnlyStore, settings: Settings, apps: Apps) -> AppUi {
    let groups = settings
        .group_signer
        .map(|signer| rooms::Groups::new(signer, store.clone()));
    let router = Router::new()
        .route(PREFIX, get(feed::everyone))
        .route("/social/assets/{name}", get(asset))
        .route("/social/following", get(feed::viewer))
        .route("/social/following/{npub}", get(feed::following))
        .route("/social/r/{target}", get(repo_page::redirect))
        .route("/social/r/{npub}/{d}", get(repo_page::maker))
        .route("/social/r/{npub}/{d}/{tab}", get(repo_page::tab))
        .route("/social/e/{id}", get(note::page))
        .route("/social/trending", get(trending::page))
        .route("/social/t", get(topics::index))
        .route("/social/t/{topic}", get(topics::page))
        .route("/social/p/{npub}", get(profile::overview))
        .route("/social/p/{npub}/{tab}", get(profile::tab))
        .route("/social/a/{naddr}", get(article::by_naddr))
        .route("/social/a/{npub}/{d}", get(article::by_key))
        .route("/social/s/{id}", get(snippet::page))
        .route("/social/events", get(events::index))
        .route("/social/ev/{target}", get(events::redirect))
        .route("/social/ev/{npub}/{d}", get(events::page))
        .route("/social/g/{id}", get(rooms::page))
        .fallback(fallback)
        .with_state(Pages {
            store,
            settings,
            apps,
            groups,
        });
    AppUi::new(PREFIX, router)
}

async fn asset(Path(name): Path<String>) -> Response {
    if name == relay_ui::stylesheet_name() {
        return relay_ui::stylesheet_response();
    }
    StatusCode::NOT_FOUND.into_response()
}

/// The top bar's `/social` part: the feed, and a form that reads the feed
/// of the key a visitor names (a `GET`, so nothing is written or kept).
fn header(viewer: Option<&PublicKey>) -> String {
    format!(
        "<nav class=\"app-nav\" aria-label=\"Social\"><a class=\"navlink\" href=\"{PREFIX}\">{}Home</a>\
         <a class=\"navlink\" href=\"{PREFIX}/trending\">{}Trending</a>\
         <a class=\"navlink\" href=\"{PREFIX}/events\">{}Events</a></nav>\
         <span class=\"grow\"></span>\
         <form class=\"viewer\" method=\"get\" action=\"{PREFIX}/following\">\
         <label for=\"viewer-npub\">Viewing as</label>\
         <input class=\"input\" id=\"viewer-npub\" name=\"npub\" placeholder=\"npub1…\" value=\"{}\" autocomplete=\"off\">\
         <button class=\"btn\" type=\"submit\">Follow feed</button></form>",
        chrome::svg(chrome::icon::HOME, ""),
        chrome::svg(chrome::icon::STAR, ""),
        chrome::svg(chrome::icon::CALENDAR, ""),
        viewer
            .map(npub)
            .map(|npub| escape(&npub))
            .unwrap_or_default(),
    )
}

/// A page with `/social`'s policy: its own, since it shows media.
fn html(
    pages: &Pages,
    status: StatusCode,
    title: &str,
    viewer: Option<&PublicKey>,
    body: &str,
) -> Response {
    let mut response = (
        status,
        Html(chrome::page(
            &pages.apps,
            App::Social,
            title,
            &header(viewer),
            body,
        )),
    )
        .into_response();
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY_VALUE),
    );
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
}

fn not_found(pages: &Pages) -> Response {
    html(
        pages,
        StatusCode::NOT_FOUND,
        "Not found",
        None,
        &format!(
            "<div class=\"box\"><div class=\"empty\"><h2>Not found</h2>\
             <p>Nothing on this relay lives at this address.</p>\
             <p><a href=\"{PREFIX}\">Back to the feed</a></p></div></div>"
        ),
    )
}

async fn fallback(State(pages): State<Pages>) -> Response {
    not_found(&pages)
}

/// The key a path or a form names: an `npub`, an `nprofile` or 64 hex.
fn parse_pubkey(text: &str) -> Option<PublicKey> {
    let text = text.trim();
    let text = text.strip_prefix("nostr:").unwrap_or(text);
    if text.len() == 64 {
        return PublicKey::from_hex(text).ok();
    }
    match Nip19::from_bech32(text).ok()? {
        Nip19::Pubkey(key) => Some(key),
        Nip19::Profile(profile) => Some(profile.public_key),
        _ => None,
    }
}

/// The event id a path names: 64 hex, a `note` or an `nevent`.
fn parse_event_id(text: &str) -> Option<EventId> {
    let text = text.strip_prefix("nostr:").unwrap_or(text);
    if text.len() == 64 {
        return EventId::from_hex(text).ok();
    }
    match Nip19::from_bech32(text).ok()? {
        Nip19::EventId(id) => Some(id),
        Nip19::Event(event) => Some(event.event_id),
        _ => None,
    }
}

/// The event with `id`, if the store holds it.
async fn event_by_id(store: &ReadOnlyStore, id: EventId) -> Option<Event> {
    store
        .query(Filter::new().id(id))
        .await
        .into_iter()
        .find(|event| event.id == id)
}

/// The cards of `repos` this relay has an announcement for, at most
/// [`CARD_LIMIT`] of them, in the order given.
async fn load_cards(
    store: &ReadOnlyStore,
    repos: impl IntoIterator<Item = RepoAddress>,
) -> HashMap<RepoAddress, RepoCard> {
    let mut cards = HashMap::new();
    let mut tried = 0;
    for repo in repos {
        if cards.contains_key(&repo) {
            continue;
        }
        if tried == CARD_LIMIT {
            break;
        }
        tried += 1;
        if let Some(card) = RepoCard::load(store, &repo).await {
            cards.insert(repo, card);
        }
    }
    cards
}

/// Whether a kind 1 note answers another (NIP-10): an `e` tag marked `root`
/// or `reply`, or an unmarked one (the deprecated positional form). A
/// `mention` is not a reply. A pure function.
fn is_reply(event: &Event) -> bool {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .any(|tag| match tag {
            [name, _] => name == "e",
            [name, _, _] => name == "e",
            [name, _, _, marker, ..] => name == "e" && marker != "mention",
            _ => false,
        })
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Kind, Tag};
    use nostr::key::Keys;
    use nostr::nips::nip19::ToBech32;

    use super::*;

    #[test]
    fn a_reply_is_an_e_tag_that_is_not_a_mention() {
        let note = |tags: &[&[&str]]| {
            EventBuilder::new(Kind::TextNote, "")
                .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
                .finalize(&Keys::generate())
                .unwrap()
        };
        let id = "a".repeat(64);
        assert!(!is_reply(&note(&[])));
        assert!(is_reply(&note(&[&["e", &id]])));
        assert!(is_reply(&note(&[&["e", &id, "", "root"]])));
        assert!(is_reply(&note(&[&["e", &id, "wss://x", "reply", &id]])));
        assert!(!is_reply(&note(&[&["e", &id, "", "mention"]])));
        assert!(!is_reply(&note(&[&["q", &id]])));
    }

    #[test]
    fn a_key_or_an_id_is_read_in_each_spelling() {
        let key = Keys::generate().public_key();
        for text in [key.to_hex(), key.to_bech32().unwrap()] {
            assert_eq!(parse_pubkey(&text), Some(key));
        }
        assert_eq!(parse_pubkey("npub1nope"), None);
        let id = EventId::from_hex(&"0".repeat(64)).unwrap();
        assert_eq!(parse_event_id(&id.to_hex()), Some(id));
        assert_eq!(parse_event_id(&id.to_bech32().unwrap()), Some(id));
        assert_eq!(parse_event_id("not-an-id"), None);
    }
}
