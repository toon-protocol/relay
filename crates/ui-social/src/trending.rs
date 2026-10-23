//! Trending repos (proposal 6 of `docs/social-for-builders-ux.md`):
//! `/social/trending`, and the feed sidebar's "Trending today".
//!
//! The score is public, and the page prints it. For a window (a day, a week
//! or thirty days), a repo's score is the number of **distinct people** who,
//! inside the window, did any of:
//!
//! | Act       | Event                                                        |
//! | --------- | ------------------------------------------------------------ |
//! | starred   | kind 7 `+` with the repo's `a` and `k=30617` (NIP-25)        |
//! | boosted   | kind 16 with the repo's `a` and `k=30617` (NIP-18)           |
//! | commented | kind 1111 with the repo in its `A` root (NIP-22)             |
//! | followed  | kind 10018 naming the repo, written in the window (NIP-51)   |
//! | forked    | kind 30617 whose `u` names the repo (NIP-34)                 |
//! | posted    | kind 1 with the repo's `a`                                   |
//!
//! The repo's owner and maintainers (ADR 0002) never count for their own
//! repo. With `?as=<npub>`, each person that key follows (its kind 3) counts
//! [`FOLLOWED_WEIGHT`] times, and a repo that key or anyone it follows
//! reported (kind 1984 with the repo's `a`) is hidden. A NIP-09 deletion has
//! already removed its event from the store, so it leaves the score too.
//!
//! Kind 10018 and 30617 are replaceable: the store keeps only each key's
//! newest list or announcement, so a follow (or fork) counts when the
//! follower's current list names the repo and was written inside the window.
//! An older follow re-saved with another change counts again; an unfollow
//! leaves no trace.
//!
//! When the store holds NIP-85 kind 30384 assertions about repos, the page
//! lists their providers under "Other rankings", and `?by=<npub>` shows one
//! provider's ranking, ordered by its `rank` tag. The relay does not compute
//! or check those ranks; it shows what the provider signed.

use std::collections::{HashMap, HashSet};

use axum::extract::{RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use nostr::event::{Event, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use nostr::types::Timestamp;
use relay_ui::ReadOnlyStore;
use relay_ui::chrome::{self, Profiles, ago, escape, npub, percent_encode, plain_name};
use ui_nip34::card::{is_star, plural};
use ui_nip34::repo::{ANNOUNCEMENT, RepoAddress, authorized_authors, first_tag};

use crate::kinds::{
    ADDRESS_ASSERTION, COMMENT, FOLLOWED_REPOS, FOLLOWS, GENERIC_REPOST, NOTE, REACTION, REPORT,
    TRUSTED_PROVIDERS,
};
use crate::topics::{self, Catalog, Rows};
use crate::{PREFIX, Pages, html, parse_pubkey};

/// How many times a person the `as` key follows counts.
pub(crate) const FOLLOWED_WEIGHT: usize = 3;
/// The bars of a row's sparkline: the window in sevenths.
pub(crate) const BUCKETS: usize = 7;
/// The tallest bar's class, `h8`.
const LEVELS: usize = 8;
/// The most acts other than notes one ranking reads.
const ACT_LIMIT: usize = 10_000;
/// The most notes one ranking reads.
const NOTE_LIMIT: usize = 3_000;
/// The most reports read for `as`.
const REPORT_LIMIT: usize = 5_000;
/// The most stars or forks read for a page's rows.
const COUNT_LIMIT: usize = 20_000;
/// The most NIP-85 assertions read.
const ASSERTION_LIMIT: usize = 2_000;
/// The rows a trending page shows.
const SHOWN: usize = 25;
/// The rows the feed's sidebar shows.
const SIDEBAR_SHOWN: usize = 5;
/// The topic chips over the list.
const TOPIC_CHIPS: usize = 8;

/// A trending window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Window {
    Day,
    Week,
    Month,
}

impl Window {
    const ALL: [Window; 3] = [Window::Day, Window::Week, Window::Month];

    /// The window `?window=` names; a week when it names none.
    pub(crate) fn parse(value: Option<&str>) -> Self {
        Self::ALL
            .into_iter()
            .find(|window| Some(window.slug()) == value)
            .unwrap_or(Window::Week)
    }

    fn slug(self) -> &'static str {
        match self {
            Self::Day => "day",
            Self::Week => "week",
            Self::Month => "month",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Day => "Today",
            Self::Week => "This week",
            Self::Month => "This month",
        }
    }

    /// The window as a row says it: "12 people this week".
    pub(crate) fn word(self) -> &'static str {
        match self {
            Self::Day => "today",
            Self::Week => "this week",
            Self::Month => "this month",
        }
    }

    /// Its length in seconds: the last 24 hours, 7 days or 30 days.
    pub(crate) fn seconds(self) -> u64 {
        match self {
            Self::Day => 86_400,
            Self::Week => 7 * 86_400,
            Self::Month => 30 * 86_400,
        }
    }

    /// What one bar of the sparkline spans.
    fn bucket_word(self) -> &'static str {
        match self {
            Self::Day => "about 3½ hours",
            Self::Week => "one day",
            Self::Month => "about 4 days",
        }
    }
}

/// The value of `name` in a query string, percent-decoded (`+` is a space).
/// A pure function.
pub(crate) fn param(query: Option<&str>, name: &str) -> Option<String> {
    query?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == name).then(|| percent_decode(value))
    })
}

/// `text` with `%XX` escapes decoded and `+` read as a space; invalid UTF-8
/// is replaced. A pure function.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        let hex = |offset: usize| {
            bytes
                .get(at + offset)
                .and_then(|digit| char::from(*digit).to_digit(16))
        };
        match (byte, hex(1), hex(2)) {
            (b'%', Some(high), Some(low)) => {
                out.push(u8::try_from(high * 16 + low).unwrap_or(b'?'));
                at += 3;
            }
            (b'+', _, _) => {
                out.push(b' ');
                at += 1;
            }
            _ => {
                out.push(byte);
                at += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `/social/trending` with these settings.
pub(crate) fn path(
    window: Window,
    viewer: Option<&PublicKey>,
    topic: Option<&str>,
    by: Option<&PublicKey>,
) -> String {
    let mut out = format!("{PREFIX}/trending?window={}", window.slug());
    if let Some(viewer) = viewer {
        out.push_str(&format!("&as={}", npub(viewer)));
    }
    if let Some(topic) = topic {
        out.push_str(&format!("&topic={}", percent_encode(topic)));
    }
    if let Some(by) = by {
        out.push_str(&format!("&by={}", npub(by)));
    }
    out
}

/// The repo a kind 30617 says it forks: its `u` tag, when that names another
/// repo's address (a `u` that is a git URL names none here). A pure function.
pub(crate) fn fork_of(announcement: &Event) -> Option<RepoAddress> {
    if announcement.kind != Kind::from(ANNOUNCEMENT) {
        return None;
    }
    let upstream = RepoAddress::parse(first_tag(announcement, "u")?)?;
    (Some(&upstream) != RepoAddress::of(announcement).as_ref()).then_some(upstream)
}

/// The repos `event` is a trending act on (the table in the module doc).
/// A pure function.
pub(crate) fn acted_on(event: &Event) -> Vec<RepoAddress> {
    match event.kind.as_u16() {
        REACTION => RepoAddress::tagged(event, "a")
            .into_iter()
            .filter(|repo| is_star(event, &repo.coordinate()))
            .collect(),
        GENERIC_REPOST if first_tag(event, "k") == Some("30617") => RepoAddress::tagged(event, "a"),
        COMMENT => RepoAddress::tagged(event, "A"),
        FOLLOWED_REPOS | NOTE => RepoAddress::tagged(event, "a"),
        ANNOUNCEMENT => fork_of(event).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// A repo's place in a window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Score {
    pub(crate) repo: RepoAddress,
    /// Distinct people who acted on it.
    pub(crate) people: usize,
    /// How many of them the `as` key follows.
    pub(crate) followed: usize,
    /// `people + followed × (FOLLOWED_WEIGHT − 1)`: what the rank sorts by.
    pub(crate) score: usize,
    /// Distinct people in each seventh of the window, oldest first.
    pub(crate) buckets: [usize; BUCKETS],
}

/// The seventh of the window `[since, since + span)` that `at` falls in; a
/// time past the end is in the last. A pure function.
fn bucket(at: u64, since: u64, span: u64) -> usize {
    let offset = u128::from(at.saturating_sub(since));
    let index = offset * BUCKETS as u128 / u128::from(span.max(1));
    usize::try_from(index).unwrap_or(BUCKETS).min(BUCKETS - 1)
}

/// The repos among `makers` (each repo with its owner and maintainers) that
/// people acted on in `events` at or after `since`, scored, the highest
/// first (a tie to more people, then to the address). A pure function.
pub(crate) fn score(
    events: &[Event],
    since: u64,
    span: u64,
    makers: &HashMap<RepoAddress, Vec<PublicKey>>,
    follows: &HashSet<PublicKey>,
) -> Vec<Score> {
    type People = HashSet<PublicKey>;
    let mut found: HashMap<RepoAddress, (People, [People; BUCKETS])> = HashMap::new();
    for event in events {
        let at = event.created_at.as_secs();
        if at < since {
            continue;
        }
        for repo in acted_on(event) {
            let Some(own) = makers.get(&repo) else {
                continue;
            };
            if own.contains(&event.pubkey) {
                continue;
            }
            let (people, buckets) = found.entry(repo).or_default();
            people.insert(event.pubkey);
            buckets[bucket(at, since, span)].insert(event.pubkey);
        }
    }
    let mut scores: Vec<Score> = found
        .into_iter()
        .map(|(repo, (people, buckets))| {
            let followed = people.iter().filter(|key| follows.contains(key)).count();
            Score {
                repo,
                people: people.len(),
                followed,
                score: people.len() + followed * (FOLLOWED_WEIGHT - 1),
                buckets: buckets.map(|bucket| bucket.len()),
            }
        })
        .collect();
    scores.sort_by(|a, b| {
        (b.score, b.people)
            .cmp(&(a.score, a.people))
            .then_with(|| a.repo.coordinate().cmp(&b.repo.coordinate()))
    });
    scores
}

/// The class of each bar, `h0` to `h8`, scaled to the tallest: a bar with
/// anyone in it is at least `h1`. A pure function.
pub(crate) fn levels(buckets: &[usize; BUCKETS]) -> [usize; BUCKETS] {
    let max = buckets.iter().copied().max().unwrap_or(0);
    buckets.map(|count| {
        if max == 0 || count == 0 {
            0
        } else {
            (count * LEVELS).div_ceil(max)
        }
    })
}

/// The `p` keys of `viewer`'s newest kind 3, once each.
async fn follows_of(store: &ReadOnlyStore, viewer: &PublicKey) -> Vec<PublicKey> {
    let lists = store
        .query(Filter::new().kind(Kind::from(FOLLOWS)).author(*viewer))
        .await;
    crate::feed::listed_keys(crate::feed::newest_list(&lists, viewer, FOLLOWS))
}

/// A ranking of a window.
#[derive(Debug, Default)]
pub(crate) struct Ranking {
    pub(crate) scores: Vec<Score>,
    /// Who the `as` key follows, when one was given.
    pub(crate) follows: Option<HashSet<PublicKey>>,
    /// Repos hidden by a report from the `as` key or someone it follows.
    pub(crate) hidden: usize,
    /// Whether a query hit its limit, so older acts may be missing.
    pub(crate) full: bool,
}

/// The ranking of `window` ending `now`, weighted by `viewer` when given.
pub(crate) async fn rank(
    store: &ReadOnlyStore,
    catalog: &Catalog,
    window: Window,
    now: u64,
    viewer: Option<&PublicKey>,
) -> Ranking {
    let since = now.saturating_sub(window.seconds());
    let mut events = store
        .query(
            Filter::new()
                .kinds(
                    [
                        REACTION,
                        GENERIC_REPOST,
                        COMMENT,
                        FOLLOWED_REPOS,
                        ANNOUNCEMENT,
                    ]
                    .map(Kind::from),
                )
                .since(Timestamp::from(since))
                .limit(ACT_LIMIT),
        )
        .await;
    let mut full = events.len() >= ACT_LIMIT;
    let notes = store
        .query(
            Filter::new()
                .kind(Kind::from(NOTE))
                .since(Timestamp::from(since))
                .limit(NOTE_LIMIT),
        )
        .await;
    full |= notes.len() >= NOTE_LIMIT;
    events.extend(notes);

    let (follows, reported) = match viewer {
        Some(viewer) => {
            let follows = follows_of(store, viewer).await;
            let mut reporters = follows.clone();
            reporters.push(*viewer);
            let reports = store
                .query(
                    Filter::new()
                        .kind(Kind::from(REPORT))
                        .authors(reporters)
                        .limit(REPORT_LIMIT),
                )
                .await;
            let reported: HashSet<RepoAddress> = reports
                .iter()
                .flat_map(|report| RepoAddress::tagged(report, "a"))
                .collect();
            (Some(follows.into_iter().collect::<HashSet<_>>()), reported)
        }
        None => (None, HashSet::new()),
    };
    let mut scores = score(
        &events,
        since,
        window.seconds(),
        &catalog.makers(),
        follows.as_ref().unwrap_or(&HashSet::new()),
    );
    let before = scores.len();
    scores.retain(|score| !reported.contains(&score.repo));
    Ranking {
        hidden: before - scores.len(),
        scores,
        follows,
        full,
    }
}

/// Distinct stargazers and distinct forks of each of `repos`: two queries.
pub(crate) async fn stars_and_forks(
    store: &ReadOnlyStore,
    repos: &[RepoAddress],
) -> HashMap<RepoAddress, (usize, usize)> {
    let mut counts = HashMap::new();
    if repos.is_empty() {
        return counts;
    }
    let coordinates: Vec<String> = repos.iter().map(RepoAddress::coordinate).collect();
    let reactions = store
        .query(
            Filter::new()
                .kind(Kind::from(REACTION))
                .custom_tags(SingleLetterTag::LOWERCASE_A, coordinates.clone())
                .limit(COUNT_LIMIT),
        )
        .await;
    let forks = store
        .query(
            Filter::new()
                .kind(Kind::from(ANNOUNCEMENT))
                .custom_tags(SingleLetterTag::LOWERCASE_U, coordinates)
                .limit(COUNT_LIMIT),
        )
        .await;
    for repo in repos {
        let coordinate = repo.coordinate();
        let stars: HashSet<PublicKey> = reactions
            .iter()
            .filter(|event| is_star(event, &coordinate))
            .map(|event| event.pubkey)
            .collect();
        let forked: HashSet<RepoAddress> = forks
            .iter()
            .filter(|event| fork_of(event).as_ref() == Some(repo))
            .filter_map(RepoAddress::of)
            .collect();
        counts.insert(repo.clone(), (stars.len(), forked.len()));
    }
    counts
}

/// A NIP-85 provider's ranking of repos: its kind 30384s whose `d` is a
/// repo's address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Provider {
    pub(crate) key: PublicKey,
    /// `(repo, rank)`, the highest rank first.
    pub(crate) ranks: Vec<(RepoAddress, u64)>,
    pub(crate) newest: Timestamp,
}

/// The providers among kind 30384 `events` that rank repos, the one ranking
/// most repos first (a tie by key). An assertion with no whole-number `rank`
/// is not a rank. A pure function.
pub(crate) fn providers(events: &[Event]) -> Vec<Provider> {
    let mut found: Vec<Provider> = Vec::new();
    for event in events
        .iter()
        .filter(|event| event.kind == Kind::from(ADDRESS_ASSERTION))
    {
        let Some(repo) = first_tag(event, "d").and_then(RepoAddress::parse) else {
            continue;
        };
        let Some(rank) = first_tag(event, "rank").and_then(|rank| rank.trim().parse::<u64>().ok())
        else {
            continue;
        };
        match found
            .iter_mut()
            .find(|provider| provider.key == event.pubkey)
        {
            Some(provider) => {
                provider.newest = provider.newest.max(event.created_at);
                if !provider.ranks.iter().any(|(held, _)| *held == repo) {
                    provider.ranks.push((repo, rank));
                }
            }
            None => found.push(Provider {
                key: event.pubkey,
                ranks: vec![(repo, rank)],
                newest: event.created_at,
            }),
        }
    }
    for provider in &mut found {
        provider
            .ranks
            .sort_by(|(a, x), (b, y)| y.cmp(x).then_with(|| a.coordinate().cmp(&b.coordinate())));
    }
    found.sort_by(|a, b| {
        b.ranks
            .len()
            .cmp(&a.ranks.len())
            .then_with(|| a.key.to_hex().cmp(&b.key.to_hex()))
    });
    found
}

/// The keys a kind 10040 names for `30384:rank`: the providers its author
/// trusts to rank addressable events. A pure function.
pub(crate) fn trusted_providers(list: &Event) -> Vec<PublicKey> {
    let mut found = Vec::new();
    for tag in list.tags.iter().map(|tag| tag.as_slice()) {
        if let [name, key, ..] = tag
            && name == "30384:rank"
            && let Ok(key) = PublicKey::from_hex(key)
            && !found.contains(&key)
        {
            found.push(key);
        }
    }
    found
}

/// What one request asked for.
struct Asked {
    window: Window,
    viewer: Option<PublicKey>,
    topic: Option<String>,
    by: Option<PublicKey>,
}

/// `/social/trending?window=day|week|month&as=<npub>&topic=<t>&by=<npub>`.
pub(crate) async fn page(State(pages): State<Pages>, RawQuery(query): RawQuery) -> Response {
    let query = query.as_deref();
    let asked = Asked {
        window: Window::parse(param(query, "window").as_deref()),
        viewer: param(query, "as").and_then(|text| parse_pubkey(&text)),
        topic: param(query, "topic").and_then(|text| topics::normalize(&text)),
        by: param(query, "by").and_then(|text| parse_pubkey(&text)),
    };
    let store = &pages.store;
    let now = u64::try_from(chrome::now()).unwrap_or(0);
    let catalog = Catalog::load(store).await;
    let assertions = store
        .query(
            Filter::new()
                .kind(Kind::from(ADDRESS_ASSERTION))
                .limit(ASSERTION_LIMIT),
        )
        .await;
    let providers = providers(&assertions);
    let trusted = match &asked.viewer {
        Some(viewer) => store
            .query(
                Filter::new()
                    .kind(Kind::from(TRUSTED_PROVIDERS))
                    .author(*viewer),
            )
            .await
            .iter()
            .max_by_key(|list| list.created_at)
            .map(trusted_providers)
            .unwrap_or_default(),
        None => Vec::new(),
    };
    let provider = asked
        .by
        .and_then(|by| providers.iter().find(|provider| provider.key == by));

    // The rows: this relay's ranking, or the provider's.
    let (ranking, mut rows): (Ranking, Vec<Row>) = match provider {
        Some(provider) => (
            Ranking::default(),
            provider
                .ranks
                .iter()
                .map(|(repo, rank)| Row {
                    repo: repo.clone(),
                    right: Right::Rank(*rank),
                    buckets: None,
                })
                .collect(),
        ),
        None => {
            let ranking = rank(store, &catalog, asked.window, now, asked.viewer.as_ref()).await;
            let rows = ranking
                .scores
                .iter()
                .map(|score| Row {
                    repo: score.repo.clone(),
                    right: Right::People(score.clone()),
                    buckets: Some(score.buckets),
                })
                .collect();
            (ranking, rows)
        }
    };
    if let Some(topic) = &asked.topic {
        rows.retain(|row| catalog.has_topic(&row.repo, topic));
    }
    rows.retain(|row| catalog.announcement(&row.repo).is_some());
    rows.truncate(SHOWN);

    let repos: Vec<RepoAddress> = rows.iter().map(|row| row.repo.clone()).collect();
    let counts = stars_and_forks(store, &repos).await;
    let mut keys: Vec<PublicKey> = asked.viewer.into_iter().chain(asked.by).collect();
    keys.extend(providers.iter().map(|provider| provider.key));
    for repo in &repos {
        if let Some(announcement) = catalog.announcement(repo) {
            keys.extend(authorized_authors(announcement));
        }
    }
    keys.sort_by_key(PublicKey::to_hex);
    keys.dedup();
    let profiles = Profiles::load(store, &keys).await;
    let names = profiles.names();
    let links = pages.links();
    let context = Rows {
        links: &links,
        catalog: &catalog,
        names,
    };

    let mut body = hero(&asked, provider, names);
    body.push_str("<div class=\"split\"><div class=\"main\">");
    if provider.is_none() {
        body.push_str(&controls(&asked, &ranking, names));
    }
    body.push_str(&topic_bar(&asked, &catalog));
    if rows.is_empty() {
        body.push_str(&format!(
            "<div class=\"box\"><div class=\"empty\">{}</div></div>",
            match (&asked.topic, provider) {
                (_, Some(_)) => "This provider ranks no repo announced here.".to_owned(),
                (Some(topic), None) => format!(
                    "Nobody acted on a repo tagged {} {}.",
                    escape(topic),
                    asked.window.word()
                ),
                (None, None) => format!("Nobody acted on a repo here {}.", asked.window.word()),
            }
        ));
    } else {
        body.push_str("<div class=\"box\"><ol class=\"trend-list\">");
        for (index, row) in rows.iter().enumerate() {
            body.push_str(&row_html(
                &context,
                row,
                index + 1,
                counts.get(&row.repo).copied().unwrap_or_default(),
                &asked,
            ));
        }
        body.push_str("</ol></div>");
    }
    if ranking.full {
        body.push_str(
            "<p class=\"muted small\">This window holds more acts than one page reads; \
             the oldest of them are not counted.</p>",
        );
    }
    body.push_str("</div><aside class=\"sidebar\">");
    body.push_str(&how_ranked(&asked, names));
    if !providers.is_empty() {
        body.push_str(&other_rankings(&asked, &providers, &trusted, names));
    }
    body.push_str(&topics_note(asked.topic.as_deref()));
    body.push_str("</aside></div>");
    let title = match &asked.topic {
        Some(topic) => format!("Trending in {topic}"),
        None => "Trending repos".to_owned(),
    };
    html(&pages, StatusCode::OK, &title, asked.viewer.as_ref(), &body)
}

/// What a row's right-hand column says.
enum Right {
    People(Score),
    Rank(u64),
}

/// A row of the list.
struct Row {
    repo: RepoAddress,
    right: Right,
    buckets: Option<[usize; BUCKETS]>,
}

/// The heading band.
fn hero(asked: &Asked, provider: Option<&Provider>, names: &HashMap<PublicKey, String>) -> String {
    let (heading, sub) = match (provider, &asked.topic) {
        (Some(provider), _) => (
            format!("Ranked by {}", escape(&plain_name(names, &provider.key))),
            "A NIP-85 provider's signed ranks of repos on this relay, highest first. \
             The relay did not compute them and cannot check how they were made."
                .to_owned(),
        ),
        (None, Some(topic)) => (
            format!("Trending in {}", escape(topic)),
            format!(
                "Repos tagged {} by their owners or labelled by others.",
                escape(topic)
            ),
        ),
        (None, None) => (
            "Trending repos".to_owned(),
            "What builders on this relay are starring, forking and talking about.".to_owned(),
        ),
    };
    format!("<div class=\"page-hero\"><h1>{heading}</h1><p class=\"muted\">{sub}</p></div>")
}

/// The window links and the weighting form.
fn controls(asked: &Asked, ranking: &Ranking, names: &HashMap<PublicKey, String>) -> String {
    let mut out =
        String::from("<div class=\"toolbar\"><nav class=\"seg-group\" aria-label=\"Window\">");
    for window in Window::ALL {
        let here = window == asked.window;
        out.push_str(&format!(
            "<a class=\"seg{}\" href=\"{}\"{}>{}</a>",
            if here { " on" } else { "" },
            escape(&path(
                window,
                asked.viewer.as_ref(),
                asked.topic.as_deref(),
                None
            )),
            if here { " aria-current=\"page\"" } else { "" },
            window.label(),
        ));
    }
    out.push_str(&format!(
        "</nav><form class=\"viewer\" method=\"get\" action=\"{PREFIX}/trending\">\
         <input type=\"hidden\" name=\"window\" value=\"{}\">",
        asked.window.slug()
    ));
    if let Some(topic) = &asked.topic {
        out.push_str(&format!(
            "<input type=\"hidden\" name=\"topic\" value=\"{}\">",
            escape(topic)
        ));
    }
    out.push_str(&format!(
        "<label for=\"weight-npub\">Weight by people</label>\
         <input class=\"input\" id=\"weight-npub\" name=\"as\" placeholder=\"npub1…\" value=\"{}\" autocomplete=\"off\">\
         <button class=\"btn\" type=\"submit\">follows</button></form></div>",
        asked
            .viewer
            .as_ref()
            .map(npub)
            .map(|npub| escape(&npub))
            .unwrap_or_default(),
    ));
    if let (Some(viewer), Some(follows)) = (&asked.viewer, &ranking.follows) {
        out.push_str(&format!(
            "<p class=\"callout small\" data-weighted>Weighted: the {} {} {} follows (kind 3) count \
             {FOLLOWED_WEIGHT}× each. {} hidden by a report (kind 1984) from them or from {}. \
             <a href=\"{}\">Remove weighting</a></p>",
            follows.len(),
            plural(follows.len(), "person", "people"),
            chrome::name(names, viewer),
            match ranking.hidden {
                0 => "No repo is".to_owned(),
                1 => "1 repo is".to_owned(),
                count => format!("{count} repos are"),
            },
            chrome::name(names, viewer),
            escape(&path(asked.window, None, asked.topic.as_deref(), None)),
        ));
    }
    out
}

/// The topic chips over the list.
fn topic_bar(asked: &Asked, catalog: &Catalog) -> String {
    let counts = catalog.counts();
    if counts.is_empty() {
        return String::new();
    }
    let mut out = String::from("<nav class=\"chips\" aria-label=\"Topics\">");
    out.push_str(&topics::chip(
        "All topics",
        None,
        &path(asked.window, asked.viewer.as_ref(), None, asked.by.as_ref()),
        asked.topic.is_none(),
    ));
    let mut shown: Vec<&str> = counts
        .iter()
        .take(TOPIC_CHIPS)
        .map(|(topic, _)| topic.as_str())
        .collect();
    if let Some(topic) = asked.topic.as_deref()
        && !shown.contains(&topic)
    {
        shown.push(topic);
    }
    for topic in shown {
        out.push_str(&topics::chip(
            topic,
            None,
            &path(
                asked.window,
                asked.viewer.as_ref(),
                Some(topic),
                asked.by.as_ref(),
            ),
            asked.topic.as_deref() == Some(topic),
        ));
    }
    out.push_str("</nav>");
    out
}

/// One ranked repo.
fn row_html(
    context: &Rows<'_>,
    row: &Row,
    rank: usize,
    (stars, forks): (usize, usize),
    asked: &Asked,
) -> String {
    let Some(announcement) = context.catalog.announcement(&row.repo) else {
        return String::new();
    };
    let mut out = format!(
        "<li class=\"trend-row\" data-repo=\"{}\"><span class=\"trend-rank\">{rank}</span>\
         <div class=\"trend-main\">{}",
        escape(&row.repo.coordinate()),
        topics::title_link(context.links, &row.repo, context.names, announcement),
    );
    if let Some(description) = first_tag(announcement, "description")
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        out.push_str(&format!("<p class=\"muted\">{}</p>", escape(description)));
    }
    out.push_str("<div class=\"meta\">");
    out.push_str(&topics::topic_chips(
        context.catalog,
        &row.repo,
        asked.topic.as_deref(),
    ));
    out.push_str(&topics::counts_html(stars, forks));
    out.push_str(&topics::built_by(
        context.names,
        &authorized_authors(announcement),
    ));
    out.push_str("</div></div>");
    if let Some(buckets) = &row.buckets {
        out.push_str(&sparkline(buckets, asked.window));
    }
    out.push_str("<div class=\"trend-people\">");
    match &row.right {
        Right::People(score) => {
            out.push_str(&format!(
                "<strong data-people=\"{0}\">{0}</strong><span class=\"muted small\">{1} {2}</span>",
                score.people,
                plural(score.people, "person", "people"),
                asked.window.word(),
            ));
            if asked.viewer.is_some() {
                out.push_str(&format!(
                    "<span class=\"muted small\" data-score=\"{0}\">score {0}, {1} followed</span>",
                    score.score, score.followed
                ));
            }
        }
        Right::Rank(rank) => out.push_str(&format!(
            "<strong data-rank=\"{rank}\">{rank}</strong><span class=\"muted small\">provider's rank</span>"
        )),
    }
    out.push_str("</div></li>");
    out
}

/// Seven bars, one per seventh of the window, as height classes.
fn sparkline(buckets: &[usize; BUCKETS], window: Window) -> String {
    let counts: Vec<String> = buckets.iter().map(usize::to_string).collect();
    let mut out = format!(
        "<span class=\"spark\" role=\"img\" aria-label=\"People per {}, oldest first: {}\" title=\"People per {}, oldest first: {}\">",
        window.bucket_word(),
        counts.join(", "),
        window.bucket_word(),
        counts.join(", "),
    );
    for level in levels(buckets) {
        out.push_str(&format!("<i class=\"h{level}\"></i>"));
    }
    out.push_str("</span>");
    out
}

/// "How this is ranked": the formula.
fn how_ranked(asked: &Asked, names: &HashMap<PublicKey, String>) -> String {
    let weight = match &asked.viewer {
        Some(viewer) => format!(
            "<li>Weighted: people {} follows (kind 3) count {FOLLOWED_WEIGHT}×, so score = people + \
             {} × followed people.</li><li>A repo {} or someone they follow reported (kind 1984 with \
             its <code>a</code>) is hidden.</li>",
            chrome::name(names, viewer),
            FOLLOWED_WEIGHT - 1,
            chrome::name(names, viewer),
        ),
        None => format!(
            "<li>Add <code>?as=&lt;npub&gt;</code> (the form above) to count the people that key \
             follows {FOLLOWED_WEIGHT}× and hide repos they reported.</li>"
        ),
    };
    format!(
        "<div class=\"side-card\" data-formula><h2>How this is ranked</h2>\
         <p class=\"small\">Score = distinct people who, in the window, starred (kind 7 <code>+</code> \
         with <code>a</code> and <code>k=30617</code>), boosted (16 with <code>k=30617</code>), \
         commented on (1111 with <code>A</code>), followed (10018), forked (30617 with <code>u</code>) \
         or posted about (1 with <code>a</code>) the repo.</p>\
         <ul class=\"bullets small\"><li>One person counts once, however many times they write.</li>\
         <li>The owner and maintainers never count for their own repo.</li>\
         <li>A deleted event (NIP-09) is gone from the store, so it no longer counts.</li>{weight}</ul>\
         <p class=\"muted small\">Follows and forks are replaceable events: the relay keeps only each \
         key's newest list or announcement, so one counts when the current one names the repo and was \
         written in the window. Every write here is paid, which puts a floor under spam; the amount is \
         not stored with the event, so money does not change the rank. Only this relay's events are \
         read.</p></div>"
    )
}

/// "Other rankings": the NIP-85 providers whose assertions are stored here.
fn other_rankings(
    asked: &Asked,
    providers: &[Provider],
    trusted: &[PublicKey],
    names: &HashMap<PublicKey, String>,
) -> String {
    let mut ordered: Vec<&Provider> = providers.iter().collect();
    ordered.sort_by_key(|provider| !trusted.contains(&provider.key));
    let mut out = String::from(
        "<div class=\"side-card\" data-other-rankings><h2>Other rankings</h2>\
         <p class=\"muted small\">Signed ranks of repos (NIP-85 kind 30384) that providers wrote to \
         this relay. Each is the provider's own calculation; the relay shows it unchecked.</p><ul>",
    );
    if asked.by.is_some() {
        out.push_str(&format!(
            "<li><a href=\"{}\">This relay's ranking</a></li>",
            escape(&path(Window::Week, asked.viewer.as_ref(), None, None))
        ));
    }
    for provider in ordered {
        out.push_str(&format!(
            "<li data-provider=\"{}\"><a href=\"{}\">{}</a> <span class=\"muted small\">{} {}, {}{}</span></li>",
            npub(&provider.key),
            escape(&path(
                asked.window,
                asked.viewer.as_ref(),
                None,
                Some(&provider.key)
            )),
            chrome::name(names, &provider.key),
            provider.ranks.len(),
            plural(provider.ranks.len(), "repo", "repos"),
            ago(provider.newest),
            if trusted.contains(&provider.key) {
                ", trusted by this key (kind 10040)"
            } else {
                ""
            },
        ));
    }
    out.push_str("</ul></div>");
    out
}

/// The sidebar's note on topics.
fn topics_note(topic: Option<&str>) -> String {
    let more = match topic {
        Some(topic) => format!(
            "<a href=\"{}\">Every repo under {}</a>",
            escape(&topics::topic_path(topic)),
            escape(topic)
        ),
        None => format!("<a href=\"{PREFIX}/t\">All topics</a>"),
    };
    format!(
        "<div class=\"side-card\"><h2>{}</h2><p class=\"muted small\">Topics come from the repo's own \
         <code>t</code> tags and from labels other people add (NIP-32, namespace <code>#t</code>), \
         shown with who added them. {more}</p></div>",
        match topic {
            Some(topic) => format!("About {}", escape(topic)),
            None => "About topics".to_owned(),
        }
    )
}

/// The feed sidebar's "Trending today": the top of today's ranking, weighted
/// by `viewer` on a following feed.
pub(crate) async fn sidebar_card(
    pages: &Pages,
    catalog: &Catalog,
    viewer: Option<&PublicKey>,
) -> String {
    let store = &pages.store;
    let now = u64::try_from(chrome::now()).unwrap_or(0);
    let ranking = rank(store, catalog, Window::Day, now, viewer).await;
    let top: Vec<&Score> = ranking.scores.iter().take(SIDEBAR_SHOWN).collect();
    let owners: Vec<PublicKey> = top.iter().map(|score| *score.repo.owner()).collect();
    let names = chrome::names(store, &owners).await;
    let links = pages.links();
    let mut rows = String::new();
    for (index, score) in top.iter().enumerate() {
        let Some(announcement) = catalog.announcement(&score.repo) else {
            continue;
        };
        rows.push_str(&format!(
            "<li><span class=\"rank\">{}</span><div class=\"who\">{}\
             <div class=\"muted small\">{} {} today</div></div></li>",
            index + 1,
            topics::title_link(&links, &score.repo, &names, announcement),
            score.people,
            plural(score.people, "person", "people"),
        ));
    }
    let list = if rows.is_empty() {
        "<p class=\"muted small\">Nobody acted on a repo here in the last day.</p>".to_owned()
    } else {
        format!("<ol class=\"ranked\">{rows}</ol>")
    };
    format!(
        "<div class=\"side-card\" data-trending><h2>Trending today</h2>{list}<p class=\"muted small\">\
         Distinct people who starred, boosted, commented on, followed, forked or posted about a repo \
         here in the last 24 hours; makers do not count for their own repos.{} \
         <a href=\"{}\">See trending</a></p></div>",
        if viewer.is_some() {
            " People this key follows count 3×."
        } else {
            ""
        },
        escape(&path(Window::Day, viewer, None, None)),
    )
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
    fn a_window_is_a_day_a_week_or_a_month_and_a_week_by_default() {
        assert_eq!(Window::parse(Some("day")), Window::Day);
        assert_eq!(Window::parse(Some("month")), Window::Month);
        assert_eq!(Window::parse(Some("year")), Window::Week);
        assert_eq!(Window::parse(None), Window::Week);
        assert_eq!(Window::Month.seconds(), 2_592_000);
    }

    #[test]
    fn a_query_parameter_is_percent_decoded() {
        let query = Some("window=day&topic=built-with%3Aclaude&as=npub1x&q=a+b&bad=%zz");
        assert_eq!(param(query, "window").as_deref(), Some("day"));
        assert_eq!(param(query, "topic").as_deref(), Some("built-with:claude"));
        assert_eq!(param(query, "q").as_deref(), Some("a b"));
        assert_eq!(param(query, "bad").as_deref(), Some("%zz"));
        assert_eq!(param(query, "missing"), None);
        assert_eq!(param(None, "window"), None);
    }

    #[test]
    fn each_act_names_the_repo_it_is_on() {
        let (owner, fan) = (Keys::generate(), Keys::generate());
        let repo = format!("30617:{}:r", owner.public_key().to_hex());
        let on = |event: Event| acted_on(&event).len();
        assert_eq!(
            on(signed(&fan, 7, 1, "+", &[&["a", &repo], &["k", "30617"]])),
            1
        );
        assert_eq!(
            on(signed(&fan, 7, 1, "-", &[&["a", &repo], &["k", "30617"]])),
            0
        );
        assert_eq!(
            on(signed(&fan, 16, 1, "", &[&["a", &repo], &["k", "30617"]])),
            1
        );
        assert_eq!(
            on(signed(&fan, 16, 1, "", &[&["a", &repo], &["k", "1"]])),
            0
        );
        assert_eq!(on(signed(&fan, 1111, 1, "", &[&["A", &repo]])), 1);
        assert_eq!(on(signed(&fan, 1111, 1, "", &[&["a", &repo]])), 0);
        assert_eq!(on(signed(&fan, 10_018, 1, "", &[&["a", &repo]])), 1);
        assert_eq!(on(signed(&fan, 1, 1, "", &[&["a", &repo]])), 1);
        assert_eq!(on(signed(&fan, 1621, 1, "", &[&["a", &repo]])), 0);
        let fork = signed(&fan, 30_617, 1, "", &[&["d", "r"], &["u", &repo]]);
        assert_eq!(fork_of(&fork), RepoAddress::parse(&repo));
        assert_eq!(on(fork), 1);
        let url = signed(
            &fan,
            30_617,
            1,
            "",
            &[&["d", "r"], &["u", "https://x/y.git"]],
        );
        assert_eq!(on(url), 0);
        let own = format!("30617:{}:r", fan.public_key().to_hex());
        assert_eq!(
            on(signed(&fan, 30_617, 1, "", &[&["d", "r"], &["u", &own]])),
            0
        );
    }

    #[test]
    fn the_score_counts_people_once_and_never_the_makers() {
        let (owner, maintainer) = (Keys::generate(), Keys::generate());
        let (a, b, c) = (Keys::generate(), Keys::generate(), Keys::generate());
        let hot = RepoAddress::new(owner.public_key(), "hot").unwrap();
        let cold = RepoAddress::new(owner.public_key(), "cold").unwrap();
        let unknown = format!("30617:{}:elsewhere", a.public_key().to_hex());
        let makers: HashMap<RepoAddress, Vec<PublicKey>> = [
            (
                hot.clone(),
                vec![owner.public_key(), maintainer.public_key()],
            ),
            (cold.clone(), vec![owner.public_key()]),
        ]
        .into();
        let star = |keys: &Keys, at: u64, repo: &str| {
            signed(keys, 7, at, "+", &[&["a", repo], &["k", "30617"]])
        };
        let (since, span) = (700, 700);
        let events = [
            // One loud key three times is one person.
            star(&a, 700, &cold.coordinate()),
            signed(&a, 1, 750, "", &[&["a", &cold.coordinate()]]),
            signed(
                &a,
                16,
                800,
                "",
                &[&["a", &cold.coordinate()], &["k", "30617"]],
            ),
            star(&a, 1_399, &hot.coordinate()),
            signed(&b, 1111, 1_000, "", &[&["A", &hot.coordinate()]]),
            // The makers, an act before the window and an unknown repo.
            star(&owner, 1_000, &hot.coordinate()),
            signed(&maintainer, 1, 1_000, "", &[&["a", &hot.coordinate()]]),
            star(&c, 699, &hot.coordinate()),
            star(&c, 900, &unknown),
        ];
        let scores = score(&events, since, span, &makers, &HashSet::new());
        assert_eq!(scores.len(), 2);
        assert_eq!((scores[0].repo.d(), scores[0].people), ("hot", 2));
        assert_eq!(scores[0].buckets, [0, 0, 0, 1, 0, 0, 1]);
        assert_eq!((scores[1].repo.d(), scores[1].people), ("cold", 1));
        assert_eq!(scores[1].buckets, [1, 1, 0, 0, 0, 0, 0]);

        // Following `b` counts b three times: hot scores 1 + 3.
        let follows: HashSet<PublicKey> = [b.public_key()].into();
        let weighted = score(&events, since, span, &makers, &follows);
        assert_eq!(weighted[0].repo.d(), "hot");
        assert_eq!((weighted[0].score, weighted[0].followed), (4, 1));
        assert_eq!((weighted[1].score, weighted[1].followed), (1, 0));

        // Following `a`, who acted on both, lifts both by two.
        let follows: HashSet<PublicKey> = [a.public_key()].into();
        let weighted = score(&events, since, span, &makers, &follows);
        assert_eq!((weighted[0].repo.d(), weighted[0].score), ("hot", 4));
        assert_eq!((weighted[1].repo.d(), weighted[1].score), ("cold", 3));
    }

    #[test]
    fn a_bar_is_scaled_to_the_tallest_and_anyone_shows() {
        assert_eq!(levels(&[0; 7]), [0; 7]);
        assert_eq!(levels(&[0, 1, 2, 4, 8, 100, 0]), [0, 1, 1, 1, 1, 8, 0]);
        assert_eq!(levels(&[1, 2, 3, 4, 5, 6, 8]), [1, 2, 3, 4, 5, 6, 8]);
        assert_eq!(bucket(0, 0, 7), 0);
        assert_eq!(bucket(6, 0, 7), 6);
        assert_eq!(bucket(99, 0, 7), 6);
    }

    #[test]
    fn a_provider_ranks_repos_by_its_rank_tag() {
        let (vertex, other) = (Keys::generate(), Keys::generate());
        let owner = Keys::generate().public_key().to_hex();
        let repo = |d: &str| format!("30617:{owner}:{d}");
        let assertion = |keys: &Keys, at: u64, d: &str, rank: &str| {
            signed(keys, 30_384, at, "", &[&["d", d], &["rank", rank]])
        };
        let events = [
            assertion(&vertex, 5, &repo("a"), "40"),
            assertion(&vertex, 9, &repo("b"), "89"),
            assertion(&vertex, 7, &repo("c"), "high"),
            assertion(&vertex, 7, "30023:x:y", "99"),
            assertion(&other, 3, &repo("a"), "10"),
        ];
        let found = providers(&events);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].key, vertex.public_key());
        assert_eq!(found[0].newest, Timestamp::from(9));
        let ranked: Vec<(&str, u64)> = found[0]
            .ranks
            .iter()
            .map(|(repo, rank)| (repo.d(), *rank))
            .collect();
        assert_eq!(ranked, [("b", 89), ("a", 40)]);
        let list = signed(
            &other,
            10_040,
            1,
            "",
            &[
                &["30384:rank", &vertex.public_key().to_hex(), "wss://x"],
                &["30382:rank", &other.public_key().to_hex()],
            ],
        );
        assert_eq!(trusted_providers(&list), [vertex.public_key()]);
    }
}
