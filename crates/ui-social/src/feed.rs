//! The feeds: `/social`, everyone on this relay, and
//! `/social/following/<npub>`, the people one key follows (kind 3 and the
//! code authors of kind 10017) and the repos it follows (kind 10018).
//!
//! A feed shows notes that start a conversation (replies are on the post's
//! page), boosts of repos, ship posts, articles, merges, and new repos; the
//! following feed adds new issues and pull requests on followed repos. The
//! sidebar's "trending today" is [`crate::trending`]'s ranking of the last
//! day, and its Topics box is [`crate::topics`]'s.

use std::collections::HashMap;

use axum::extract::{Path, RawQuery, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use nostr::event::{Event, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use nostr::types::Timestamp;
use relay_ui::chrome::{self, Profiles, escape, npub};
use ui_nip34::repo::RepoAddress;

use crate::demo::{self, Demo};
use crate::kinds::{
    ANNOUNCEMENT, APP_HANDLER, ARTICLE, FOLLOWED_REPOS, FOLLOWS, GENERIC_REPOST, GIT_AUTHORS,
    ISSUE, NOTE, PULL_REQUEST, STATUS_APPLIED,
};
use crate::mute::Mutes;
use crate::post::{self, View};
use crate::ship;
use crate::topics::{self, Catalog};
use crate::trending;
use crate::{PREFIX, Pages, content, html, load_cards, parse_pubkey};

/// What a feed reads.
const FEED_KINDS: [u16; 6] = [
    NOTE,
    GENERIC_REPOST,
    ARTICLE,
    APP_HANDLER,
    STATUS_APPLIED,
    ANNOUNCEMENT,
];
/// The most events a feed asks the store for, before it drops replies.
const FEED_QUERY: usize = 120;
/// The most posts a feed page shows.
const SHOWN: usize = 40;
/// The most follows a following feed reads from.
const FOLLOW_LIMIT: usize = 2_000;

/// Whether a feed shows `event` at all, before any repo is loaded: a note
/// that is not a reply, a boost of a repo, and the other feed kinds. A pure
/// function.
fn admits(event: &Event) -> bool {
    match event.kind.as_u16() {
        NOTE => !crate::is_reply(event),
        GENERIC_REPOST => {
            ui_nip34::repo::first_tag(event, "k") == Some("30617")
                && !RepoAddress::tagged(event, "a").is_empty()
        }
        ARTICLE | ANNOUNCEMENT | ISSUE | PULL_REQUEST => true,
        APP_HANDLER => ship::app_site(event).is_some(),
        STATUS_APPLIED => !RepoAddress::tagged(event, "a").is_empty(),
        _ => false,
    }
}

/// `?until=<unix seconds>` from a query string: where an older page starts.
fn until(query: Option<&str>) -> Option<Timestamp> {
    query?
        .split('&')
        .find_map(|pair| pair.strip_prefix("until="))
        .and_then(|value| value.parse::<u64>().ok())
        .map(Timestamp::from)
}

/// `/social`: every post on this relay.
pub(crate) async fn everyone(State(pages): State<Pages>, RawQuery(query): RawQuery) -> Response {
    let mut filter = Filter::new()
        .kinds(FEED_KINDS.map(Kind::from))
        .limit(FEED_QUERY);
    if let Some(until) = until(query.as_deref()) {
        filter = filter.until(until);
    }
    let events = pages.store.query(filter).await;
    let lens = crate::viewer::as_key(query.as_deref());
    let mutes = match &lens {
        Some(key) => Mutes::load(&pages.store, key).await,
        None => Mutes::default(),
    };
    let mut intro = "<p class=\"muted small\">Everything builders posted to this relay: notes, launches, \
                 articles, merges and new repos. Every post here was a paid write; this page writes nothing.</p>"
        .to_owned();
    if let Some(key) = &lens {
        intro.push_str(&muted_line(&pages, key, &mutes).await);
    }
    let body = feed_page(&pages, events, None, &intro, &mutes, lens.as_ref()).await;
    html(
        &pages,
        crate::Nav::Home,
        StatusCode::OK,
        "Social",
        None,
        &body,
    )
}

/// `/social/following?npub=…`: the form's answer, a redirect to the feed of
/// the key it names.
pub(crate) async fn viewer(State(pages): State<Pages>, RawQuery(query): RawQuery) -> Response {
    let named = query
        .as_deref()
        .unwrap_or("")
        .split('&')
        .find_map(|pair| pair.strip_prefix("npub="))
        .map(|value| value.replace("%3A", ":").replace("%3a", ":"))
        .and_then(|value| parse_pubkey(&value));
    match named {
        Some(key) => (
            StatusCode::SEE_OTHER,
            [(
                header::LOCATION,
                format!("{PREFIX}/following/{}", npub(&key)),
            )],
        )
            .into_response(),
        None => html(
            &pages,
            crate::Nav::Home,
            StatusCode::BAD_REQUEST,
            "Not a key",
            None,
            &format!(
                "<div class=\"box\"><div class=\"empty\"><h2>That is not a Nostr key</h2>\
                 <p>Enter an npub (or 64 hex) to read the feed of the people and repos it follows.</p>\
                 <p><a href=\"{PREFIX}\">Back to the feed</a></p></div></div>"
            ),
        ),
    }
}

/// The newest of `events` of `kind` by `author`.
pub(crate) fn newest_list<'a>(
    events: &'a [Event],
    author: &PublicKey,
    kind: u16,
) -> Option<&'a Event> {
    ui_nip34::repo::newest(
        events
            .iter()
            .filter(|event| event.pubkey == *author && event.kind == Kind::from(kind)),
    )
}

/// The `p` keys of a list, once each.
pub(crate) fn listed_keys(list: Option<&Event>) -> Vec<PublicKey> {
    let mut keys: Vec<PublicKey> = Vec::new();
    for tag in list.into_iter().flat_map(|list| list.tags.iter()) {
        if let [name, value, ..] = tag.as_slice()
            && name == "p"
            && let Ok(key) = PublicKey::from_hex(value)
            && !keys.contains(&key)
        {
            keys.push(key);
        }
    }
    keys
}

/// `/social/following/<npub>`: the feed of what `npub` follows.
pub(crate) async fn following(
    State(pages): State<Pages>,
    Path(target): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    let Some(viewer) = parse_pubkey(&target) else {
        return crate::not_found(&pages);
    };
    let store = &pages.store;
    let lists = store
        .query(
            Filter::new()
                .kinds([
                    Kind::from(FOLLOWS),
                    Kind::from(GIT_AUTHORS),
                    Kind::from(FOLLOWED_REPOS),
                ])
                .author(viewer),
        )
        .await;
    let mut people = listed_keys(newest_list(&lists, &viewer, FOLLOWS));
    for key in listed_keys(newest_list(&lists, &viewer, GIT_AUTHORS)) {
        if !people.contains(&key) {
            people.push(key);
        }
    }
    people.truncate(FOLLOW_LIMIT);
    let repos: Vec<RepoAddress> = newest_list(&lists, &viewer, FOLLOWED_REPOS)
        .map(|list| RepoAddress::tagged(list, "a"))
        .unwrap_or_default();
    let until = until(query.as_deref());
    let mut events = Vec::new();
    if !people.is_empty() {
        let mut filter = Filter::new()
            .kinds(FEED_KINDS.map(Kind::from))
            .authors(people.clone())
            .limit(FEED_QUERY);
        if let Some(until) = until {
            filter = filter.until(until);
        }
        events.extend(store.query(filter).await);
    }
    if !repos.is_empty() {
        let mut filter = Filter::new()
            .kinds(
                FEED_KINDS
                    .into_iter()
                    .chain([ISSUE, PULL_REQUEST])
                    .map(Kind::from),
            )
            .custom_tags(
                SingleLetterTag::LOWERCASE_A,
                repos.iter().map(RepoAddress::coordinate),
            )
            .limit(FEED_QUERY);
        if let Some(until) = until {
            filter = filter.until(until);
        }
        events.extend(store.query(filter).await);
    }
    let names = Profiles::load(store, &[viewer]).await;
    let intro = format!(
        "<p class=\"muted small\">What {} follows: {} {} (kind 3 and 10017) and {} {} (kind 10018). \
         This page writes nothing; follows are changed in a client.</p>",
        chrome::person(&pages.apps, names.names(), &viewer),
        people.len(),
        if people.len() == 1 {
            "person"
        } else {
            "people"
        },
        repos.len(),
        if repos.len() == 1 { "repo" } else { "repos" },
    );
    let mutes = Mutes::load(store, &viewer).await;
    let mut intro = intro;
    intro.push_str(&muted_line(&pages, &viewer, &mutes).await);
    let body = feed_page(&pages, events, Some(&viewer), &intro, &mutes, None).await;
    html(
        &pages,
        crate::Nav::Home,
        StatusCode::OK,
        "Following",
        Some(&viewer),
        &body,
    )
}

/// Whether `viewer`'s mutes drop `event` from a feed: [`Mutes::hides`], or a
/// boost of a repo whose owner is muted. A pure function.
fn hidden(mutes: &Mutes, event: &Event) -> bool {
    mutes.hides(event)
        || (event.kind == Kind::from(GENERIC_REPOST)
            && RepoAddress::tagged(event, "a")
                .iter()
                .any(|repo| mutes.hides_key(repo.owner())))
}

/// The line that says whose mute list a feed applies.
async fn muted_line(pages: &Pages, viewer: &PublicKey, mutes: &Mutes) -> String {
    let names = Profiles::load(&pages.store, &[*viewer]).await;
    let (people, hashtags) = mutes.counts();
    format!(
        "<p class=\"muted small\" data-muted>Read as {}: what their mute list (NIP-51 kind 10000) \
         names is left out here, {people} {} and {hashtags} {} among it. Only its public items are \
         read.</p>",
        chrome::name(names.names(), viewer),
        if people == 1 { "person" } else { "people" },
        if hashtags == 1 { "hashtag" } else { "hashtags" },
    )
}

/// A feed's page: its tabs, its posts and the sidebar.
async fn feed_page(
    pages: &Pages,
    mut events: Vec<Event>,
    viewer: Option<&PublicKey>,
    intro: &str,
    mutes: &Mutes,
    lens: Option<&PublicKey>,
) -> String {
    let store = &pages.store;
    events.sort_by(|a, b| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
    events.dedup_by_key(|event| event.id);
    let full = events.len() >= FEED_QUERY;
    let shown: Vec<Event> = events
        .into_iter()
        .filter(admits)
        .filter(|event| !hidden(mutes, event))
        .take(SHOWN)
        .collect();

    // Everything the posts point at, read once.
    let mut repos: Vec<RepoAddress> = Vec::new();
    for event in &shown {
        for repo in post::repos_of(event) {
            if !repos.contains(&repo) {
                repos.push(repo);
            }
        }
    }
    let cards = load_cards(store, repos).await;
    let target_ids: Vec<nostr::event::EventId> = shown
        .iter()
        .filter(|event| event.kind == Kind::from(STATUS_APPLIED))
        .flat_map(|event| event.tags.iter())
        .filter_map(|tag| match tag.as_slice() {
            [name, id, ..] if name == "e" => nostr::event::EventId::from_hex(id).ok(),
            _ => None,
        })
        .collect();
    let targets: HashMap<String, Event> = if target_ids.is_empty() {
        HashMap::new()
    } else {
        store
            .query(Filter::new().ids(target_ids))
            .await
            .into_iter()
            .map(|event| (event.id.to_hex(), event))
            .collect()
    };
    let mut keys: Vec<PublicKey> = Vec::new();
    for key in shown
        .iter()
        .flat_map(|event| {
            std::iter::once(event.pubkey).chain(content::mentioned_keys(&event.content))
        })
        .chain(targets.values().map(|target| target.pubkey))
        .chain(viewer.copied())
    {
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
    let demos = demos(pages, &view, &shown).await;
    let view = View {
        demos: &demos,
        ..view
    };

    let mut body = String::from("<div class=\"split\"><div class=\"main\">");
    let mut tabs = vec![
        chrome::Tab::new("Everyone here", chrome::icon::HOME, PREFIX).current(viewer.is_none()),
    ];
    if let Some(viewer) = viewer {
        tabs.push(
            chrome::Tab::new(
                "Following",
                chrome::icon::PERSON,
                format!("{PREFIX}/following/{}", npub(viewer)),
            )
            .current(true),
        );
    }
    body.push_str(&chrome::tabs("Feed", &tabs));
    body.push_str(intro);
    let mut drawn = 0;
    body.push_str("<div class=\"feed\">");
    for event in &shown {
        let item = match event.kind.as_u16() {
            NOTE | APP_HANDLER => match view.ship(event) {
                Some(repo) => Some(post::ship_card(&view, event, &repo)),
                None if event.kind.as_u16() == NOTE => Some(post::note_card(&view, event)),
                None => None,
            },
            GENERIC_REPOST => post::repost_card(&view, event),
            ARTICLE => Some(post::article_card(&view, event)),
            STATUS_APPLIED => post::merged_row(&view, event, &targets),
            ANNOUNCEMENT => post::announced_card(&view, event),
            ISSUE | PULL_REQUEST => post::thread_row(&view, event),
            _ => None,
        };
        if let Some(item) = item {
            body.push_str(&item);
            drawn += 1;
        }
    }
    body.push_str("</div>");
    if drawn == 0 {
        body.push_str("<div class=\"box\"><div class=\"empty\">Nothing to show yet.</div></div>");
    }
    if (full || shown.len() == SHOWN)
        && let Some(oldest) = shown.last()
    {
        let base = match viewer {
            Some(viewer) => format!("{PREFIX}/following/{}", npub(viewer)),
            None => PREFIX.to_owned(),
        };
        body.push_str(&format!(
            "<div class=\"pager\"><a class=\"btn\" href=\"{}?until={}{}\">Older</a></div>",
            escape(&base),
            oldest.created_at.as_secs().saturating_sub(1),
            escape(&crate::viewer::as_query(lens, false)),
        ));
    }
    body.push_str("</div><aside class=\"sidebar\">");
    let catalog = Catalog::load(store).await;
    body.push_str(&trending::sidebar_card(pages, &catalog, viewer.or(lens)).await);
    body.push_str(&topics::sidebar_card(&catalog));
    body.push_str(&crate::events::coming_up(pages).await);
    body.push_str(
        "<div class=\"side-card\"><h2>How this works</h2><p class=\"muted small\">Stars, follows, \
         replies and launches are written by a Nostr client and paid for one write each; this page \
         only reads them. A star is a <code>+</code> reaction on a repo, a follow is the repo in a \
         kind 10018 list, and a launch is a maker's note tagged <code>ship</code>.</p></div>",
    );
    body.push_str("</aside></div>");
    body
}

/// The live demos of the repos whose cards are drawn, a launch's own site
/// first ([`crate::demo`]).
async fn demos(pages: &Pages, view: &View<'_>, shown: &[Event]) -> HashMap<RepoAddress, Demo> {
    let launches: Vec<(&Event, RepoAddress)> = shown
        .iter()
        .filter_map(|event| view.ship(event).map(|repo| (event, repo)))
        .collect();
    demo::for_cards(&pages.store, view.cards, &launches, pages.gateway()).await
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;

    use super::*;

    fn signed(keys: &Keys, kind: u16, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn a_feed_starts_conversations_and_shows_repo_boosts_only() {
        let keys = Keys::generate();
        let repo = format!("30617:{}:r", keys.public_key().to_hex());
        let id = "a".repeat(64);
        assert!(admits(&signed(&keys, 1, &[])));
        assert!(!admits(&signed(&keys, 1, &[&["e", &id, "", "reply"]])));
        assert!(admits(&signed(
            &keys,
            16,
            &[&["a", &repo], &["k", "30617"]]
        )));
        assert!(!admits(&signed(&keys, 16, &[&["e", &id], &["k", "1"]])));
        assert!(!admits(&signed(&keys, 31_990, &[&["a", &repo]])));
        assert!(!admits(&signed(&keys, 1631, &[&["e", &id]])));
        assert!(!admits(&signed(&keys, 7, &[&["a", &repo]])));
    }

    #[test]
    fn an_older_page_starts_before_the_last_post() {
        assert_eq!(until(Some("until=99")), Some(Timestamp::from(99)));
        assert_eq!(until(Some("x=1&until=nope")), None);
        assert_eq!(until(None), None);
    }

    #[test]
    fn a_follow_list_names_each_key_once() {
        let keys = Keys::generate();
        let friend = Keys::generate().public_key().to_hex();
        let list = signed(&keys, 3, &[&["p", &friend], &["p", &friend], &["p", "bad"]]);
        assert_eq!(listed_keys(Some(&list)).len(), 1);
        assert!(listed_keys(None).is_empty());
    }
}
