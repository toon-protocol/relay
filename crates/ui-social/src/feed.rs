//! The feeds: `/social`, everyone on this relay, and
//! `/social/following/<npub>`, the people one key follows (kind 3 and the
//! code authors of kind 10017) and the repos it follows (kind 10018).
//!
//! A feed shows notes that start a conversation (replies are on the post's
//! page), boosts of repos, ship posts, articles, merges, and new repos; the
//! following feed adds new issues and pull requests on followed repos. The
//! sidebar's "trending today" counts distinct people, never events.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, RawQuery, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use nostr::event::{Event, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use nostr::types::Timestamp;
use relay_ui::chrome::{self, Profiles, escape, npub};
use ui_nip34::card::RepoCard;
use ui_nip34::repo::RepoAddress;

use crate::kinds::{
    ANNOUNCEMENT, APP_HANDLER, ARTICLE, COMMENT, FOLLOWED_REPOS, FOLLOWS, GENERIC_REPOST,
    GIT_AUTHORS, ISSUE, NOTE, PULL_REQUEST, REACTION, STATUS_APPLIED,
};
use crate::post::{self, View};
use crate::ship::{self, Site};
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
/// What "trending today" counts: acts that name a repo.
const TRENDING_KINDS: [u16; 7] = [
    NOTE,
    REACTION,
    GENERIC_REPOST,
    COMMENT,
    ISSUE,
    PULL_REQUEST,
    ARTICLE,
];
/// The window "today" is.
const DAY: u64 = 86_400;

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

/// The repos trending among `events`: for each repo named in an `a` or `A`
/// tag, the number of distinct keys that named it, most first (a tie by
/// address), at most `top`. A pure function.
pub(crate) fn trending(events: &[Event], top: usize) -> Vec<(RepoAddress, usize)> {
    let mut people: HashMap<RepoAddress, HashSet<PublicKey>> = HashMap::new();
    for event in events {
        let mut repos = RepoAddress::tagged(event, "a");
        repos.extend(RepoAddress::tagged(event, "A"));
        for repo in repos {
            people.entry(repo).or_default().insert(event.pubkey);
        }
    }
    let mut ranked: Vec<(RepoAddress, usize)> = people
        .into_iter()
        .map(|(repo, keys)| (repo, keys.len()))
        .collect();
    ranked.sort_by(|(a, x), (b, y)| y.cmp(x).then_with(|| a.coordinate().cmp(&b.coordinate())));
    ranked.truncate(top);
    ranked
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
    let intro = "<p class=\"muted small\">Everything builders posted to this relay: notes, launches, \
                 articles, merges and new repos. Every post here was a paid write; this page writes nothing.</p>";
    let body = feed_page(&pages, events, None, intro).await;
    html(&pages, StatusCode::OK, "Social", None, &body)
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
fn newest_list<'a>(events: &'a [Event], author: &PublicKey, kind: u16) -> Option<&'a Event> {
    ui_nip34::repo::newest(
        events
            .iter()
            .filter(|event| event.pubkey == *author && event.kind == Kind::from(kind)),
    )
}

/// The `p` keys of a list, once each.
fn listed_keys(list: Option<&Event>) -> Vec<PublicKey> {
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
    let body = feed_page(&pages, events, Some(&viewer), &intro).await;
    html(&pages, StatusCode::OK, "Following", Some(&viewer), &body)
}

/// A feed's page: its tabs, its posts and the sidebar.
async fn feed_page(
    pages: &Pages,
    mut events: Vec<Event>,
    viewer: Option<&PublicKey>,
    intro: &str,
) -> String {
    let store = &pages.store;
    events.sort_by(|a, b| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
    events.dedup_by_key(|event| event.id);
    let full = events.len() >= FEED_QUERY;
    let shown: Vec<Event> = events.into_iter().filter(admits).take(SHOWN).collect();

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
            "<div class=\"pager\"><a class=\"btn\" href=\"{}?until={}\">Older</a></div>",
            escape(&base),
            oldest.created_at.as_secs().saturating_sub(1)
        ));
    }
    body.push_str("</div><aside class=\"sidebar\">");
    body.push_str(&trending_card(pages, &view).await);
    body.push_str(
        "<div class=\"side-card\"><h2>How this works</h2><p class=\"muted small\">Stars, follows, \
         replies and launches are written by a Nostr client and paid for one write each; this page \
         only reads them. A star is a <code>+</code> reaction on a repo, a follow is the repo in a \
         kind 10018 list, and a launch is a maker's note tagged <code>ship</code>.</p></div>",
    );
    body.push_str("</aside></div>");
    body
}

/// The live demos of the ship posts among `shown`.
async fn demos(pages: &Pages, view: &View<'_>, shown: &[Event]) -> HashMap<RepoAddress, Site> {
    let mut demos = HashMap::new();
    if pages.gateway().is_none() {
        return demos;
    }
    for event in shown {
        let Some(repo) = view.ship(event) else {
            continue;
        };
        if demos.contains_key(&repo) {
            continue;
        }
        let site = match ship::app_site(event) {
            Some(site) => Some(site),
            None => match view.cards.get(&repo) {
                Some(card) => ship::demo(&pages.store, card).await,
                None => None,
            },
        };
        if let Some(site) = site {
            demos.insert(repo, site);
        }
    }
    demos
}

/// "Trending today": the repos the most distinct people named in the last
/// day, by the formula the card states.
async fn trending_card(pages: &Pages, view: &View<'_>) -> String {
    let since = Timestamp::from(
        u64::try_from(chrome::now())
            .unwrap_or(0)
            .saturating_sub(DAY),
    );
    let events = pages
        .store
        .query(
            Filter::new()
                .kinds(TRENDING_KINDS.map(Kind::from))
                .since(since)
                .limit(2_000),
        )
        .await;
    let ranked = trending(&events, 10);
    let mut cards: HashMap<RepoAddress, RepoCard> = HashMap::new();
    let mut rows = String::new();
    let mut rank = 0;
    for (repo, people) in ranked {
        if rank == 5 {
            break;
        }
        let card = match view.cards.get(&repo) {
            Some(card) => card.clone(),
            None => match RepoCard::load(&pages.store, &repo).await {
                Some(card) => card,
                None => continue,
            },
        };
        rank += 1;
        let href = view
            .links
            .social_path(&repo)
            .unwrap_or_else(|| repo.repo_path());
        rows.push_str(&format!(
            "<li><span class=\"rank\">{rank}</span><div class=\"who\"><a class=\"strong\" href=\"{}\">{}</a>\
             <div class=\"muted small\">{people} {} today</div></div></li>",
            escape(&href),
            escape(&card.title()),
            if people == 1 { "person" } else { "people" },
        ));
        cards.insert(repo, card);
    }
    let list = if rows.is_empty() {
        "<p class=\"muted small\">No repo was named in the last day.</p>".to_owned()
    } else {
        format!("<ol class=\"ranked\">{rows}</ol>")
    };
    format!(
        "<div class=\"side-card\"><h2>Trending today</h2>{list}<p class=\"muted small\">\
         Distinct people who named a repo in a note, reaction, boost, comment, issue, pull request \
         or article on this relay in the last 24 hours. People, not events.</p></div>"
    )
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
    fn trending_counts_people_not_events() {
        let owner = Keys::generate().public_key().to_hex();
        let hot = format!("30617:{owner}:hot");
        let cold = format!("30617:{owner}:cold");
        let (a, b) = (Keys::generate(), Keys::generate());
        let events = [
            // One loud key ten times is one person.
            signed(&a, 7, &[&["a", &cold]]),
            signed(&a, 1, &[&["a", &cold]]),
            signed(&a, 16, &[&["a", &cold]]),
            signed(&a, 7, &[&["a", &hot]]),
            signed(&b, 1111, &[&["A", &hot]]),
            signed(&b, 1, &[&["a", "30023:x:y"]]),
        ];
        let ranked = trending(&events, 5);
        assert_eq!(ranked[0], (RepoAddress::parse(&hot).unwrap(), 2));
        assert_eq!(ranked[1], (RepoAddress::parse(&cold).unwrap(), 1));
        assert_eq!(ranked.len(), 2);
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
