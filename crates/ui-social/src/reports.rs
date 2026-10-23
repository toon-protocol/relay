//! Reports (NIP-56, kind 1984) on a repo, an app and what they ship, read
//! for the visitor and never acted on (proposal 18 of
//! `docs/social-for-builders-ux.md`).
//!
//! A report names what it reports in a tag whose third entry is the report
//! type: `e` for an event (a release, an asset, a post), `x` for a blob's
//! hash (an asset's file), and, by the convention `docs/apps/social.md`
//! states (NIP-56 itself names only `p`, `e` and `x`), `a` for an
//! addressable event such as a repo or an app. The type is one of NIP-56's
//! seven; anything else reads as `other`, and a missing one falls back to
//! the `p` tag's.
//!
//! What a page does with them, with or without a lens:
//!
//! - with `?as=<npub>`, a banner says how many of the people that key
//!   follows reported it, by type, and links to the list;
//! - without one, a neutral line says how many people reported it here.
//!
//! Nothing is hidden because of a report: strangers can report anything,
//! and removal stays the operator's blocklist (`docs/retention.md`). A
//! person's reports count once.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use nostr::event::{Event, EventId, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use relay_ui::ReadOnlyStore;
use relay_ui::chrome::{Profiles, ago, avatar, escape, name, plain_name};
use ui_nip34::card::{RepoCard, plural};
use ui_nip34::repo::RepoAddress;

use crate::kinds::{APP_HANDLER, NOTE, REPORT};
use crate::viewer::{self, as_key};
use crate::{PREFIX, Pages, apps, html, not_found, parse_pubkey};

/// NIP-56's report types.
const TYPES: [&str; 7] = [
    "malware",
    "impersonation",
    "illegal",
    "spam",
    "nudity",
    "profanity",
    "other",
];
/// The most reports one query reads.
const LIMIT: usize = 2_000;
/// The most maker posts about a repo whose reports a repo page reads.
const MAKER_POSTS: usize = 200;

/// What reports may name, each with how a reader calls it.
#[derive(Debug, Clone, Default)]
pub(crate) struct Targets {
    addresses: HashMap<String, String>,
    events: HashMap<String, String>,
    blobs: HashMap<String, String>,
}

impl Targets {
    /// An addressable event, by its `<kind>:<pubkey>:<d>`.
    pub(crate) fn address(&mut self, coordinate: String, label: String) {
        self.addresses.entry(coordinate).or_insert(label);
    }

    /// An event, by its id.
    pub(crate) fn event(&mut self, id: EventId, label: String) {
        self.events.entry(id.to_hex()).or_insert(label);
    }

    /// A blob, by its SHA-256 in hex.
    pub(crate) fn blob(&mut self, hash: &str, label: String) {
        let hash = hash.trim().to_lowercase();
        if hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()) {
            self.blobs.entry(hash).or_insert(label);
        }
    }

    fn is_empty(&self) -> bool {
        self.addresses.is_empty() && self.events.is_empty() && self.blobs.is_empty()
    }

    /// What `name`'s tag `value` names, if it is one of these.
    fn label(&self, name: &str, value: &str) -> Option<&str> {
        match name {
            "a" => self.addresses.get(value),
            "e" => self.events.get(value),
            "x" => self.blobs.get(&value.to_lowercase()),
            _ => None,
        }
        .map(String::as_str)
    }
}

/// A report type as NIP-56 names it; anything else is `other`.
fn known(value: &str) -> &'static str {
    let value = value.trim().to_lowercase();
    TYPES
        .into_iter()
        .find(|known| *known == value)
        .unwrap_or("other")
}

/// What `report` says about `targets`, if it names one: the report type
/// and how the reader calls what it names. A pure function.
pub(crate) fn reason<'a>(report: &Event, targets: &'a Targets) -> Option<(&'static str, &'a str)> {
    if report.kind != Kind::from(REPORT) {
        return None;
    }
    let tags = || report.tags.iter().map(|tag| tag.as_slice());
    let (named, label) = tags().find_map(|tag| match tag {
        [name, value, rest @ ..] => targets
            .label(name, value)
            .map(|label| (rest.first().map(String::as_str), label)),
        _ => None,
    })?;
    let from_p = || {
        tags().find_map(|tag| match tag {
            [name, _, kind, ..] if name == "p" && !kind.trim().is_empty() => Some(kind.as_str()),
            _ => None,
        })
    };
    let kind = named
        .filter(|kind| !kind.trim().is_empty())
        .or_else(from_p)
        .map_or("other", known);
    Some((kind, label))
}

/// How many distinct people reported, and how many of them by each type,
/// most first; only people in `within` when it is given. A pure function.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Tally {
    pub(crate) people: usize,
    pub(crate) by_type: Vec<(&'static str, usize)>,
}

pub(crate) fn tally(
    reports: &[Event],
    targets: &Targets,
    within: Option<&HashSet<PublicKey>>,
) -> Tally {
    let mut people: HashSet<PublicKey> = HashSet::new();
    let mut by_type: HashMap<&'static str, HashSet<PublicKey>> = HashMap::new();
    for report in reports {
        if within.is_some_and(|keys| !keys.contains(&report.pubkey)) {
            continue;
        }
        let Some((kind, _)) = reason(report, targets) else {
            continue;
        };
        people.insert(report.pubkey);
        by_type.entry(kind).or_default().insert(report.pubkey);
    }
    let mut ranked: Vec<(&'static str, usize)> = by_type
        .into_iter()
        .map(|(kind, keys)| (kind, keys.len()))
        .collect();
    ranked.sort_by(|(a, x), (b, y)| y.cmp(x).then_with(|| a.cmp(b)));
    Tally {
        people: people.len(),
        by_type: ranked,
    }
}

/// The reports in the store that name one of `targets`, newest first.
pub(crate) async fn load(store: &ReadOnlyStore, targets: &Targets) -> Vec<Event> {
    if targets.is_empty() {
        return Vec::new();
    }
    let mut found: Vec<Event> = Vec::new();
    for (tag, values) in [
        (SingleLetterTag::LOWERCASE_A, &targets.addresses),
        (SingleLetterTag::LOWERCASE_E, &targets.events),
        (SingleLetterTag::LOWERCASE_X, &targets.blobs),
    ] {
        if values.is_empty() {
            continue;
        }
        found.extend(
            store
                .query(
                    Filter::new()
                        .kind(Kind::from(REPORT))
                        .custom_tags(tag, values.keys().cloned())
                        .limit(LIMIT),
                )
                .await,
        );
    }
    found.retain(|report| reason(report, targets).is_some());
    found.sort_by(|a, b| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
    found.dedup_by_key(|report| report.id);
    found
}

/// `malware (2), spam (1)`.
fn types_line(tally: &Tally) -> String {
    tally
        .by_type
        .iter()
        .map(|(kind, count)| format!("{kind} ({count})"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The line a page shows about its reports: with a lens (the viewer and
/// the keys it follows), an attention banner when people it follows
/// reported this; in any case, when anyone did, a neutral count. `link` is
/// the reports list. Empty when nobody reported anything.
pub(crate) fn banner(
    reports: &[Event],
    targets: &Targets,
    lens: Option<(&PublicKey, &HashSet<PublicKey>)>,
    names: &HashMap<PublicKey, String>,
    link: Option<&str>,
) -> String {
    let everyone = tally(reports, targets, None);
    if everyone.people == 0 {
        return String::new();
    }
    let see = |text: &str| match link {
        Some(link) => format!("<a href=\"{}\">{text}</a>", escape(link)),
        None => String::new(),
    };
    let mut out = String::new();
    if let Some((viewer, follows)) = lens {
        let theirs = tally(reports, targets, Some(follows));
        if theirs.people > 0 {
            out.push_str(&format!(
                "<div class=\"report-banner\" role=\"note\" data-reports-banner=\"{}\"><span class=\"text\">\
                 <span class=\"strong\">{} {} {} follows</span> reported this (kind 1984): {}.</span>{}</div>",
                theirs.people,
                theirs.people,
                plural(theirs.people, "person", "people"),
                name(names, viewer),
                escape(&types_line(&theirs)),
                see("See the reports"),
            ));
        }
    }
    out.push_str(&format!(
        "<p class=\"muted small\" data-reports=\"{}\">{} {} on this relay. {}</p>",
        everyone.people,
        everyone.people,
        plural(everyone.people, "report", "reports"),
        see("Read them"),
    ));
    out
}

/// The list of reports, those from the lens's follows first.
fn list(
    reports: &[Event],
    targets: &Targets,
    follows: Option<&HashSet<PublicKey>>,
    names: &HashMap<PublicKey, String>,
) -> String {
    if reports.is_empty() {
        return "<div class=\"box\"><div class=\"empty\">No reports on this relay.</div></div>"
            .to_owned();
    }
    let mut ordered: Vec<&Event> = reports.iter().collect();
    ordered.sort_by_key(|report| !follows.is_some_and(|keys| keys.contains(&report.pubkey)));
    let mut out = String::from("<div class=\"box\"><ul class=\"rows\">");
    for report in ordered {
        let Some((kind, label)) = reason(report, targets) else {
            continue;
        };
        let followed = follows.is_some_and(|keys| keys.contains(&report.pubkey));
        out.push_str(&format!(
            "<li class=\"report\" data-id=\"{}\"><div class=\"by\">{}<div class=\"who\"><div class=\"line\">\
             <a href=\"{}\">{}</a><span class=\"chip chip-report\">{kind}</span>{}</div>\
             <span class=\"muted small\">reported {} &middot; {}</span></div></div>{}</li>",
            report.id.to_hex(),
            avatar(&plain_name(names, &report.pubkey), true),
            relay_ui::chrome::profile_path(&report.pubkey),
            name(names, &report.pubkey),
            if followed {
                "<span class=\"chip\">followed</span>"
            } else {
                ""
            },
            escape(label),
            ago(report.created_at),
            if report.content.trim().is_empty() {
                String::new()
            } else {
                format!(
                    "<p class=\"text\">{}</p>",
                    escape(&crate::content::clip(&report.content, 600))
                )
            },
        ));
    }
    out.push_str("</ul></div>");
    out
}

/// A reports page: what it covers, the lens, the counts and the list.
async fn reports_page(
    pages: &Pages,
    title: &str,
    covers: &str,
    back: &str,
    targets: &Targets,
    query: Option<&str>,
) -> Response {
    let store = &pages.store;
    let viewer = as_key(query);
    let follows: Option<HashSet<PublicKey>> = match &viewer {
        Some(viewer) => Some(viewer::follows(store, viewer).await.into_iter().collect()),
        None => None,
    };
    let found = load(store, targets).await;
    let mut keys: Vec<PublicKey> = Vec::new();
    for key in found.iter().map(|report| report.pubkey).chain(viewer) {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let profiles = Profiles::load(store, &keys).await;
    let names = profiles.names();
    let everyone = tally(&found, targets, None);
    let mut body = format!(
        "<h1 class=\"page-title\">Reports on {}</h1><p class=\"muted small\">Reports (NIP-56, kind 1984) \
         that name {}. They inform; nothing here is \
         hidden because of them, and removing anything is the relay operator's call. \
         <a href=\"{}\">Back</a></p>",
        escape(title),
        escape(covers),
        escape(back),
    );
    body.push_str(&viewer::as_form(viewer.as_ref(), "Viewing as"));
    if let (Some(viewer), Some(follows)) = (&viewer, &follows) {
        let theirs = tally(&found, targets, Some(follows));
        body.push_str(&format!(
            "<p data-reports-from-follows=\"{0}\">{0} {1} {2} follows reported it{3}.</p>",
            theirs.people,
            plural(theirs.people, "person", "people"),
            name(names, viewer),
            if theirs.people == 0 {
                String::new()
            } else {
                format!(": {}", escape(&types_line(&theirs)))
            },
        ));
    }
    body.push_str(&format!(
        "<p class=\"muted small\" data-reports=\"{}\">{} {} on this relay{}.</p>",
        everyone.people,
        everyone.people,
        plural(everyone.people, "person reported it", "people reported it"),
        if everyone.people == 0 {
            String::new()
        } else {
            format!(": {}", escape(&types_line(&everyone)))
        },
    ));
    body.push_str(&list(&found, targets, follows.as_ref(), names));
    html(
        pages,
        StatusCode::OK,
        &format!("Reports · {title}"),
        None,
        &body,
    )
}

/// What a report about a repo may name: the repo, and the app listings its
/// makers publish for it with their releases and files.
pub(crate) async fn repo_targets(store: &ReadOnlyStore, card: &RepoCard) -> Targets {
    let mut targets = Targets::default();
    targets.address(
        card.address().coordinate(),
        format!("the repo {}", card.name()),
    );
    for app in apps::apps_of_repo(store, card).await {
        apps::add_targets(store, &app, &mut targets).await;
    }
    // The makers' own posts about it: its launches and release notes.
    let posts = store
        .query(
            Filter::new()
                .kinds([Kind::from(NOTE), Kind::from(APP_HANDLER)])
                .authors(card.authorized().to_vec())
                .custom_tag(SingleLetterTag::LOWERCASE_A, card.address().coordinate())
                .limit(MAKER_POSTS),
        )
        .await;
    for post in posts {
        targets.event(post.id, "a maker's post about it".to_owned());
    }
    targets
}

/// `/social/r/<npub>/<d>/reports`.
pub(crate) async fn repo(
    State(pages): State<Pages>,
    Path((owner, d)): Path<(String, String)>,
    RawQuery(query): RawQuery,
) -> Response {
    let Some(repo) = parse_pubkey(&owner).and_then(|owner| RepoAddress::new(owner, &d)) else {
        return not_found(&pages);
    };
    let Some(card) = RepoCard::load(&pages.store, &repo).await else {
        return not_found(&pages);
    };
    let targets = repo_targets(&pages.store, &card).await;
    let back = format!("{PREFIX}/r/{}", repo.path_segments());
    reports_page(
        &pages,
        &card.title(),
        "the repo, its makers' posts about it, the app listings they publish for it, their releases or their files",
        &back,
        &targets,
        query.as_deref(),
    )
    .await
}

/// `/social/app/<npub>/<d>/reports`.
pub(crate) async fn app(
    State(pages): State<Pages>,
    Path((owner, d)): Path<(String, String)>,
    RawQuery(query): RawQuery,
) -> Response {
    let Some(address) = apps::address_of_path(&owner, &d) else {
        return not_found(&pages);
    };
    let Some(app) = apps::find(&pages.store, &address).await else {
        return not_found(&pages);
    };
    let mut targets = Targets::default();
    apps::add_targets(&pages.store, &app, &mut targets).await;
    reports_page(
        &pages,
        app.name(),
        "the app, its releases or their files",
        &app.path(),
        &targets,
        query.as_deref(),
    )
    .await
}

/// A post page's report line: reports on the post and, for a launch
/// (`launched` is the repo's card), on the repo it launches, linking to
/// that repo's reports.
pub(crate) async fn post_banner(
    store: &ReadOnlyStore,
    event: &Event,
    launched: Option<&RepoCard>,
    links: &ui_nip34::card::Links,
    query: Option<&str>,
) -> String {
    let mut targets = match launched {
        Some(card) => repo_targets(store, card).await,
        None => Targets::default(),
    };
    targets.event(event.id, "this post".to_owned());
    let found = load(store, &targets).await;
    if found.is_empty() {
        return String::new();
    }
    let viewer = as_key(query);
    let follows: Option<HashSet<PublicKey>> = match &viewer {
        Some(viewer) => Some(viewer::follows(store, viewer).await.into_iter().collect()),
        None => None,
    };
    let names = match &viewer {
        Some(viewer) => Profiles::load(store, &[*viewer]).await.into_names(),
        None => HashMap::new(),
    };
    let link = launched
        .and_then(|card| links.social_path(card.address()))
        .map(|base| format!("{base}/reports{}", viewer::as_query(viewer.as_ref(), true)));
    banner(
        &found,
        &targets,
        viewer.as_ref().zip(follows.as_ref()),
        &names,
        link.as_deref(),
    )
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;

    use super::*;

    fn report(keys: &Keys, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(REPORT), "")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .finalize(keys)
            .unwrap()
    }

    fn targets() -> (Targets, String, String, String) {
        let owner = Keys::generate().public_key().to_hex();
        let repo = format!("30617:{owner}:tidepool");
        let asset = EventId::from_hex(&"ab".repeat(32)).unwrap();
        let hash = "cd".repeat(32);
        let mut targets = Targets::default();
        targets.address(repo.clone(), "the repo".to_owned());
        targets.event(asset, "an asset".to_owned());
        targets.blob(&hash, "an asset".to_owned());
        (targets, repo, asset.to_hex(), hash)
    }

    #[test]
    fn a_report_names_its_target_and_type_in_one_tag() {
        let (targets, repo, asset, hash) = targets();
        let reporter = Keys::generate();
        let owner = "0".repeat(64);
        assert_eq!(
            reason(
                &report(&reporter, &[&["a", &repo, "malware"], &["p", &owner]]),
                &targets
            ),
            Some(("malware", "the repo"))
        );
        assert_eq!(
            reason(
                &report(
                    &reporter,
                    &[&["x", &hash, "malware"], &["e", &asset, "malware"]]
                ),
                &targets
            ),
            Some(("malware", "an asset"))
        );
        // The type falls back to the p tag's, then to "other"; an unknown one is "other".
        assert_eq!(
            reason(
                &report(&reporter, &[&["e", &asset], &["p", &owner, "spam"]]),
                &targets
            ),
            Some(("spam", "an asset"))
        );
        assert_eq!(
            reason(&report(&reporter, &[&["a", &repo, "scary"]]), &targets),
            Some(("other", "the repo"))
        );
        assert_eq!(
            reason(
                &report(&reporter, &[&["e", &"ef".repeat(32), "spam"]]),
                &targets
            ),
            None
        );
    }

    #[test]
    fn a_tally_counts_people_and_may_be_limited_to_follows() {
        let (targets, repo, asset, _) = targets();
        let (friend, stranger) = (Keys::generate(), Keys::generate());
        let reports = vec![
            report(&friend, &[&["a", &repo, "malware"]]),
            report(&friend, &[&["e", &asset, "malware"]]),
            report(&stranger, &[&["a", &repo, "spam"]]),
            report(&stranger, &[&["a", &repo, "malware"]]),
        ];
        let all = tally(&reports, &targets, None);
        assert_eq!(all.people, 2);
        assert_eq!(all.by_type, vec![("malware", 2), ("spam", 1)]);
        let follows: HashSet<PublicKey> = [friend.public_key()].into();
        let theirs = tally(&reports, &targets, Some(&follows));
        assert_eq!((theirs.people, theirs.by_type), (1, vec![("malware", 1)]));
    }

    #[test]
    fn the_banner_needs_a_lens_and_a_followed_reporter() {
        let (targets, repo, _, _) = targets();
        let (viewer, friend, stranger) = (Keys::generate(), Keys::generate(), Keys::generate());
        let names = HashMap::new();
        let strangers = vec![report(&stranger, &[&["a", &repo, "spam"]])];
        let follows: HashSet<PublicKey> = [friend.public_key()].into();
        let lens = Some((&viewer.public_key(), &follows));
        let page = banner(&strangers, &targets, lens, &names, Some("/r"));
        assert!(!page.contains("data-reports-banner"), "{page}");
        assert!(page.contains("1 report on this relay"));
        let mut both = strangers.clone();
        both.push(report(&friend, &[&["a", &repo, "malware"]]));
        let page = banner(&both, &targets, lens, &names, Some("/r"));
        assert!(page.contains("data-reports-banner=\"1\""));
        assert!(page.contains("malware (1)"));
        assert!(page.contains("2 reports on this relay"));
        assert_eq!(banner(&[], &targets, lens, &names, None), "");
    }
}
