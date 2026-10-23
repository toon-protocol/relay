//! Bounties and goals, `/social/bounties` (proposal 15 of
//! `docs/social-for-builders-ux.md`).
//!
//! A bounty is a NIP-99 classified listing (kind 30402) tagged `t` `bounty`
//! that names a repo (`a` `30617:…`) or an issue (`e` of a kind 1621). It
//! shows its title and its `price` as written (amount, currency and any
//! frequency; nothing here converts or checks it) and one status:
//!
//! - **Resolved** when its issue's honoured status is applied, a kind 1631
//!   from the repo's owner, a maintainer or the issue's author (ADR 0002,
//!   rule 3);
//! - else **Claimed** when the listing's own NIP-99 `status` says `sold`
//!   (or `claimed`);
//! - else **Open**.
//!
//! A goal is a NIP-75 zap goal (kind 9041) that names a repo in an `a`
//! tag. Its target and description show; its progress does not, because
//! NIP-75 tallies Lightning zap receipts (kind 9735), which TOON does not
//! produce, and the relay never checks a payment. So no bar is drawn.

use std::collections::{HashMap, HashSet};

use axum::extract::{RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use nostr::event::{Event, EventId, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use relay_ui::ReadOnlyStore;
use relay_ui::chrome::{Profiles, ago, escape, name, percent_encode};
use ui_nip34::card::{RepoCard, plural};
use ui_nip34::repo::{RepoAddress, first_tag, supersedes};

use crate::apps::Address;
use crate::content::{clip, first_line};
use crate::kinds::{CLASSIFIED, ISSUE, ZAP_GOAL};
use crate::viewer::param;
use crate::{PREFIX, Pages, html, load_cards};

/// The most listings or goals one query reads.
const LIMIT: usize = 500;

/// Where a bounty stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Open,
    Claimed,
    Resolved,
}

impl Status {
    const ALL: [Status; 3] = [Status::Open, Status::Claimed, Status::Resolved];

    fn slug(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Claimed => "claimed",
            Self::Resolved => "resolved",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::Open => "Open",
            Self::Claimed => "Claimed",
            Self::Resolved => "Resolved",
        }
    }

    fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|status| status.slug() == slug)
    }
}

/// A listing's status, given whether its issue is resolved: see the
/// module's rules. A pure function.
pub(crate) fn status_of(listing: &Event, resolved: bool) -> Status {
    if resolved {
        return Status::Resolved;
    }
    match first_tag(listing, "status").map(|status| status.trim().to_lowercase()) {
        Some(status) if status == "sold" || status == "claimed" => Status::Claimed,
        _ => Status::Open,
    }
}

/// Whether a listing is tagged `t` `bounty`.
fn is_bounty(listing: &Event) -> bool {
    listing.kind == Kind::from(CLASSIFIED)
        && listing
            .tags
            .iter()
            .map(|tag| tag.as_slice())
            .any(|tag| matches!(tag, [name, value, ..] if name == "t" && value.eq_ignore_ascii_case("bounty")))
}

/// The price as written: `50 USDC`, `15 EUR per month`. A pure function.
pub(crate) fn price(listing: &Event) -> Option<String> {
    let tag = listing
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find(|tag| tag.first().is_some_and(|name| name == "price"))?;
    let amount = tag
        .get(1)
        .map(|amount| amount.trim())
        .filter(|a| !a.is_empty())?;
    let mut out = amount.to_owned();
    if let Some(currency) = tag.get(2).map(|c| c.trim()).filter(|c| !c.is_empty()) {
        out.push(' ');
        out.push_str(currency);
    }
    if let Some(frequency) = tag.get(3).map(|f| f.trim()).filter(|f| !f.is_empty()) {
        out.push_str(" per ");
        out.push_str(frequency);
    }
    Some(out)
}

/// A goal's target, from its `amount` in millisats. A pure function.
pub(crate) fn target(goal: &Event) -> Option<String> {
    let millisats: u64 = first_tag(goal, "amount")?.trim().parse().ok()?;
    Some(if millisats.is_multiple_of(1_000) {
        format!("{} sats", grouped(millisats / 1_000))
    } else {
        format!("{} millisats", grouped(millisats))
    })
}

/// `21000` as `21,000`.
fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (at, digit) in digits.chars().enumerate() {
        if at > 0 && (digits.len() - at).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// One bounty, its links resolved.
#[derive(Debug, Clone)]
pub(crate) struct Bounty {
    listing: Event,
    repo: Option<RepoAddress>,
    issue: Option<Event>,
    status: Status,
}

/// Every bounty on this relay that names a repo or an issue, newest first,
/// with its status.
pub(crate) async fn load(store: &ReadOnlyStore) -> (Vec<Bounty>, HashMap<RepoAddress, RepoCard>) {
    let mut held: HashMap<Address, Event> = HashMap::new();
    for listing in store
        .query(
            Filter::new()
                .kind(Kind::from(CLASSIFIED))
                .custom_tag(SingleLetterTag::LOWERCASE_T, "bounty")
                .limit(LIMIT),
        )
        .await
        .into_iter()
        .filter(is_bounty)
    {
        let Some(address) = Address::of(&listing) else {
            continue;
        };
        match held.get(&address) {
            Some(current) if !supersedes(&listing, current) => {}
            _ => {
                held.insert(address, listing);
            }
        }
    }
    let listings: Vec<Event> = held.into_values().collect();
    let issue_ids: Vec<EventId> = listings
        .iter()
        .flat_map(|listing| listing.tags.iter())
        .filter_map(|tag| match tag.as_slice() {
            [name, id, ..] if name == "e" => EventId::from_hex(id).ok(),
            _ => None,
        })
        .collect();
    let issues: HashMap<EventId, Event> = if issue_ids.is_empty() {
        HashMap::new()
    } else {
        store
            .query(Filter::new().kind(Kind::from(ISSUE)).ids(issue_ids))
            .await
            .into_iter()
            .map(|issue| (issue.id, issue))
            .collect()
    };
    let mut bounties: Vec<Bounty> = listings
        .into_iter()
        .filter_map(|listing| {
            let issue = listing
                .tags
                .iter()
                .find_map(|tag| match tag.as_slice() {
                    [name, id, ..] if name == "e" => {
                        EventId::from_hex(id).ok().and_then(|id| issues.get(&id))
                    }
                    _ => None,
                })
                .cloned();
            let repo = RepoAddress::tagged(&listing, "a")
                .into_iter()
                .next()
                .or_else(|| {
                    issue
                        .as_ref()
                        .and_then(|issue| RepoAddress::tagged(issue, "a").into_iter().next())
                });
            (repo.is_some() || issue.is_some()).then_some(Bounty {
                listing,
                repo,
                issue,
                status: Status::Open,
            })
        })
        .collect();
    let mut repos: Vec<RepoAddress> = Vec::new();
    for repo in bounties.iter().filter_map(|bounty| bounty.repo.clone()) {
        if !repos.contains(&repo) {
            repos.push(repo);
        }
    }
    let cards = load_cards(store, repos).await;
    // The statuses of the issues, from honoured authors only, so a
    // stranger's cannot crowd one out of the query's limit.
    let mut authors: HashSet<PublicKey> = HashSet::new();
    for bounty in &bounties {
        if let Some(issue) = &bounty.issue {
            authors.insert(issue.pubkey);
            if let Some(card) = bounty.repo.as_ref().and_then(|repo| cards.get(repo)) {
                authors.extend(card.authorized().iter().copied());
            }
        }
    }
    let statuses: Vec<Event> = if authors.is_empty() {
        Vec::new()
    } else {
        store
            .query(
                Filter::new()
                    .kinds(ui_nip34::STATUS_KINDS.map(Kind::from))
                    .events(
                        bounties
                            .iter()
                            .filter_map(|b| b.issue.as_ref().map(|i| i.id)),
                    )
                    .authors(authors)
                    .limit(LIMIT * 4),
            )
            .await
    };
    for bounty in &mut bounties {
        let resolved = bounty.issue.as_ref().is_some_and(|issue| {
            let authorized = bounty
                .repo
                .as_ref()
                .and_then(|repo| cards.get(repo))
                .map(|card| card.authorized().to_vec())
                .unwrap_or_default();
            ui_nip34::is_applied(&authorized, issue, &statuses)
        });
        bounty.status = status_of(&bounty.listing, resolved);
    }
    bounties.sort_by(|a, b| {
        (b.listing.created_at, a.listing.id).cmp(&(a.listing.created_at, b.listing.id))
    });
    (bounties, cards)
}

/// How many open bounties name `repo` (or one of its issues).
pub(crate) async fn open_count(store: &ReadOnlyStore, repo: &RepoAddress) -> usize {
    let (bounties, _) = load(store).await;
    bounties
        .iter()
        .filter(|bounty| bounty.status == Status::Open && bounty.repo.as_ref() == Some(repo))
        .count()
}

/// The repo page's sidebar card: its open bounties, or nothing.
pub(crate) async fn repo_sidebar(store: &ReadOnlyStore, repo: &RepoAddress) -> String {
    let open = open_count(store, repo).await;
    if open == 0 {
        return String::new();
    }
    format!(
        "<div class=\"side-card\" data-open-bounties=\"{open}\"><h2>Bounties</h2><p>\
         <a href=\"{PREFIX}/bounties?repo={}\">{open} open {}</a> {} here.</p></div>",
        percent_encode(&repo.coordinate()),
        plural(open, "bounty", "bounties"),
        plural(open, "links", "link"),
    )
}

/// The chip of a status.
fn chip(status: Status) -> String {
    format!(
        "<span class=\"chip chip-{}\" data-status=\"{}\">{}</span>",
        status.slug(),
        status.slug(),
        status.label()
    )
}

/// One bounty in the list.
fn row(
    pages: &Pages,
    bounty: &Bounty,
    cards: &HashMap<RepoAddress, RepoCard>,
    names: &HashMap<PublicKey, String>,
) -> String {
    let links = pages.links();
    let listing = &bounty.listing;
    let title = first_tag(listing, "title")
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| first_line(&listing.content, 100));
    let card = bounty.repo.as_ref().and_then(|repo| cards.get(repo));
    let issue_link = match (&bounty.issue, &bounty.repo) {
        (Some(issue), Some(repo)) => links
            .repo_path(repo)
            .map(|base| format!("{base}/issues/{}", issue.id.to_hex())),
        _ => None,
    };
    let repo_link = bounty
        .repo
        .as_ref()
        .and_then(|repo| links.social_path(repo).or_else(|| links.repo_path(repo)));
    let title_href = issue_link.clone().or_else(|| repo_link.clone());
    let title_html = match &title_href {
        Some(href) => format!(
            "<a class=\"strong\" href=\"{}\">{}</a>",
            escape(href),
            escape(&title)
        ),
        None => format!("<span class=\"strong\">{}</span>", escape(&title)),
    };
    let mut meta: Vec<String> = Vec::new();
    if let Some(repo) = &bounty.repo {
        let label = card
            .map(RepoCard::title)
            .unwrap_or_else(|| repo.d().to_owned());
        meta.push(match &repo_link {
            Some(href) => format!("<a href=\"{}\">{}</a>", escape(href), escape(&label)),
            None => escape(&label),
        });
    }
    if let Some(issue) = &bounty.issue {
        let subject = first_tag(issue, "subject")
            .map(str::to_owned)
            .unwrap_or_else(|| first_line(&issue.content, 60));
        meta.push(match &issue_link {
            Some(href) => format!(
                "issue <a href=\"{}\">{}</a>",
                escape(href),
                escape(&subject)
            ),
            None => format!("issue {}", escape(&subject)),
        });
    }
    meta.push(format!("posted by {}", name(names, &listing.pubkey)));
    meta.push(ago(listing.created_at));
    let summary = first_tag(listing, "summary")
        .map(|summary| clip(summary, 240))
        .filter(|summary| !summary.trim().is_empty());
    format!(
        "<article class=\"bounty\" data-id=\"{}\" data-bounty-status=\"{}\"><div class=\"who\">\
         <div class=\"line\">{}{title_html}</div><span class=\"muted small\">{}</span>{}</div>\
         <div class=\"amount\">{}</div></article>",
        listing.id.to_hex(),
        bounty.status.slug(),
        chip(bounty.status),
        meta.join(" &middot; "),
        summary
            .map(|summary| format!("<p class=\"muted small\">{}</p>", escape(&summary)))
            .unwrap_or_default(),
        match price(listing) {
            Some(price) => format!(
                "<span class=\"strong\" data-price>{}</span><span class=\"muted small\">asked</span>",
                escape(&price)
            ),
            None => "<span class=\"muted small\">no price given</span>".to_owned(),
        },
    )
}

/// The goals that name a repo (only `repo`'s when it is given), newest
/// first.
async fn goals(store: &ReadOnlyStore, repo: Option<&RepoAddress>) -> Vec<(Event, RepoAddress)> {
    let mut filter = Filter::new().kind(Kind::from(ZAP_GOAL)).limit(LIMIT);
    if let Some(repo) = repo {
        filter = filter.custom_tag(SingleLetterTag::LOWERCASE_A, repo.coordinate());
    }
    let mut found: Vec<(Event, RepoAddress)> = store
        .query(filter)
        .await
        .into_iter()
        .filter_map(|goal| {
            let tagged = RepoAddress::tagged(&goal, "a");
            let named = match repo {
                Some(repo) => tagged.into_iter().find(|each| each == repo),
                None => tagged.into_iter().next(),
            }?;
            Some((goal, named))
        })
        .collect();
    found.sort_by(|(a, _), (b, _)| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
    found
}

/// One goal in the sidebar.
fn goal_card(
    pages: &Pages,
    goal: &Event,
    repo: &RepoAddress,
    card: Option<&RepoCard>,
    names: &HashMap<PublicKey, String>,
) -> String {
    let links = pages.links();
    let label = card
        .map(RepoCard::title)
        .unwrap_or_else(|| repo.d().to_owned());
    let repo_html = match links.social_path(repo) {
        Some(href) => format!("<a href=\"{}\">{}</a>", escape(&href), escape(&label)),
        None => escape(&label),
    };
    let heading = first_tag(goal, "summary")
        .map(str::trim)
        .filter(|summary| !summary.is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| first_line(&goal.content, 100));
    let mut out = format!(
        "<div class=\"side-card\" data-goal=\"{}\"><span class=\"muted small\">Goal on {repo_html}</span>\
         <h2>{}</h2>",
        goal.id.to_hex(),
        escape(&heading)
    );
    if !goal.content.trim().is_empty() && goal.content.trim() != heading {
        out.push_str(&format!("<p>{}</p>", escape(&clip(&goal.content, 400))));
    }
    if let Some(target) = target(goal) {
        out.push_str(&format!(
            "<p>Target <span class=\"strong\" data-target>{}</span></p>",
            escape(&target)
        ));
    }
    if let Some(closes) = first_tag(goal, "closed_at").and_then(|at| at.trim().parse::<i64>().ok())
    {
        out.push_str(&format!(
            "<p class=\"muted small\">Closes {}</p>",
            relay_ui::chrome::date(closes)
        ));
    }
    out.push_str(&format!(
        "<p class=\"muted small\">Set by {} &middot; {}</p><p class=\"muted small\" data-no-progress>\
         Progress is not shown. NIP-75 tallies Lightning zap receipts (kind 9735), which TOON does \
         not produce, and this relay never checks a payment. A count would need a receipt the payer \
         writes, which no NIP defines yet.</p></div>",
        name(names, &goal.pubkey),
        ago(goal.created_at),
    ));
    out
}

/// `/social/bounties`: every bounty, `?status=` one status, `?repo=` one
/// repo's; the goals beside them.
pub(crate) async fn page(State(pages): State<Pages>, RawQuery(query): RawQuery) -> Response {
    let store = &pages.store;
    let wanted = param(query.as_deref(), "status").and_then(|slug| Status::from_slug(&slug));
    let repo = param(query.as_deref(), "repo").and_then(|value| RepoAddress::parse(&value));
    let (bounties, mut cards) = load(store).await;
    let in_repo: Vec<&Bounty> = bounties
        .iter()
        .filter(|bounty| repo.is_none() || bounty.repo == repo)
        .collect();
    let shown: Vec<&Bounty> = in_repo
        .iter()
        .copied()
        .filter(|bounty| wanted.is_none_or(|status| bounty.status == status))
        .collect();
    let goals = goals(store, repo.as_ref()).await;
    let missing: Vec<RepoAddress> = goals
        .iter()
        .map(|(_, repo)| repo.clone())
        .chain(repo.clone())
        .filter(|repo| !cards.contains_key(repo))
        .collect();
    cards.extend(load_cards(store, missing).await);
    let mut keys: Vec<PublicKey> = Vec::new();
    for key in shown
        .iter()
        .map(|bounty| bounty.listing.pubkey)
        .chain(goals.iter().map(|(goal, _)| goal.pubkey))
    {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let profiles = Profiles::load(store, &keys).await;
    let names = profiles.names();

    let scope = repo
        .as_ref()
        .map(|repo| format!("&repo={}", percent_encode(&repo.coordinate())))
        .unwrap_or_default();
    let mut body = String::from(
        "<div class=\"split\"><div class=\"main\"><h1 class=\"page-title\">Bounties</h1>",
    );
    match repo.as_ref().and_then(|repo| cards.get(repo)) {
        Some(card) => body.push_str(&format!(
            "<p class=\"muted small\">Bounties on <span class=\"strong\">{}</span>. \
             <a href=\"{PREFIX}/bounties\">All bounties</a></p>",
            escape(&card.title())
        )),
        None => body.push_str(
            "<p class=\"muted small\">Paid asks tied to a repo or an issue: NIP-99 listings tagged \
             <code>bounty</code>. A bounty is resolved when its issue is, by the repo's owner, a \
             maintainer or the issue's author. The price is the poster's; this relay moves no money \
             and checks none.</p>",
        ),
    }
    body.push_str("<nav class=\"seg-links\" aria-label=\"Status\">");
    let all_href = match &repo {
        Some(repo) => format!(
            "{PREFIX}/bounties?repo={}",
            percent_encode(&repo.coordinate())
        ),
        None => format!("{PREFIX}/bounties"),
    };
    body.push_str(&format!(
        "<a class=\"seg{}\" href=\"{}\">All <span class=\"count\">{}</span></a>",
        if wanted.is_none() { " current" } else { "" },
        escape(&all_href),
        in_repo.len()
    ));
    for status in Status::ALL {
        let count = in_repo
            .iter()
            .filter(|bounty| bounty.status == status)
            .count();
        body.push_str(&format!(
            "<a class=\"seg{}\" href=\"{PREFIX}/bounties?status={}{}\">{} <span class=\"count\">{count}</span></a>",
            if wanted == Some(status) { " current" } else { "" },
            status.slug(),
            escape(&scope),
            status.label(),
        ));
    }
    body.push_str("</nav>");
    if shown.is_empty() {
        body.push_str("<div class=\"box\"><div class=\"empty\">No bounties here.</div></div>");
    } else {
        body.push_str("<div class=\"box bounties\">");
        for bounty in &shown {
            body.push_str(&row(&pages, bounty, &cards, names));
        }
        body.push_str("</div>");
    }
    body.push_str("</div><aside class=\"sidebar\">");
    if goals.is_empty() {
        body.push_str(
            "<div class=\"side-card\"><h2>Goals</h2><p class=\"muted small\">No funding goal (NIP-75, \
             kind 9041) names a repo here yet.</p></div>",
        );
    }
    for (goal, goal_repo) in goals.iter().take(10) {
        body.push_str(&goal_card(
            &pages,
            goal,
            goal_repo,
            cards.get(goal_repo),
            names,
        ));
    }
    body.push_str(
        "<div class=\"side-card\"><h2>Posting a bounty</h2><p class=\"muted small\">This page writes \
         nothing. A client publishes a kind 30402 with <code>t</code> <code>bounty</code>, a \
         <code>price</code>, and an <code>a</code> to the repo or an <code>e</code> to the issue, as \
         one paid write. Paying the bounty happens outside this relay.</p></div></aside></div>",
    );
    html(&pages, StatusCode::OK, "Bounties", None, &body)
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;

    use super::*;

    fn listing(tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(CLASSIFIED), "Fix it")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .finalize(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn a_resolved_issue_wins_then_the_listings_own_status() {
        let sold = listing(&[&["status", "sold"]]);
        let active = listing(&[&["status", "active"]]);
        let bare = listing(&[]);
        assert_eq!(status_of(&sold, true), Status::Resolved);
        assert_eq!(status_of(&sold, false), Status::Claimed);
        assert_eq!(
            status_of(&listing(&[&["status", "Claimed"]]), false),
            Status::Claimed
        );
        assert_eq!(status_of(&active, false), Status::Open);
        assert_eq!(status_of(&bare, false), Status::Open);
    }

    #[test]
    fn a_price_is_shown_as_written() {
        assert_eq!(
            price(&listing(&[&["price", "50", "USDC"]])).as_deref(),
            Some("50 USDC")
        );
        assert_eq!(
            price(&listing(&[&["price", "15", "EUR", "month"]])).as_deref(),
            Some("15 EUR per month")
        );
        assert_eq!(price(&listing(&[&["price", ""]])), None);
        assert_eq!(price(&listing(&[])), None);
    }

    #[test]
    fn only_a_listing_tagged_bounty_is_one() {
        assert!(is_bounty(&listing(&[&["t", "Bounty"]])));
        assert!(!is_bounty(&listing(&[&["t", "electronics"]])));
    }

    #[test]
    fn a_goal_target_is_read_from_millisats() {
        let goal = |amount: &str| {
            EventBuilder::new(Kind::from(ZAP_GOAL), "")
                .tags([Tag::parse(["amount", amount]).unwrap()])
                .finalize(&Keys::generate())
                .unwrap()
        };
        assert_eq!(target(&goal("210000")).as_deref(), Some("210 sats"));
        assert_eq!(target(&goal("500000000")).as_deref(), Some("500,000 sats"));
        assert_eq!(target(&goal("1500")).as_deref(), Some("1,500 millisats"));
        assert_eq!(target(&goal("lots")), None);
    }
}
