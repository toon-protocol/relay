//! A repository's issues and patches/PRs: lists, threads and honoured status.

use std::collections::HashMap;

use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use nostr::event::{Event, EventId, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use relay_ui::ReadOnlyStore;
use relay_ui::chrome::Apps;

use super::markup::{Tab, ago, avatar, icon, name, plain_name, repo_header, svg};
use super::{
    ANNOUNCEMENT, authorized_authors, escape, first_tag, html, names, newest, not_found, repo_path,
    resolve_repo,
};

const ISSUE: u16 = 1621;
const PATCH: u16 = 1617;
const PULL_REQUEST: u16 = 1618;
const PULL_REQUEST_UPDATE: u16 = 1619;
const COMMENT: u16 = 1111;
const LEGACY_COMMENT: u16 = 1622;
const STATUS_OPEN: u16 = 1630;
const STATUS_APPLIED: u16 = 1631;
const STATUS_CLOSED: u16 = 1632;
const STATUS_DRAFT: u16 = 1633;
/// The most events one query here reads.
const LIMIT: usize = 500;

/// Where a thread stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Open,
    Applied,
    Closed,
    Draft,
}

impl Status {
    fn from_kind(kind: Kind) -> Option<Self> {
        match kind.as_u16() {
            STATUS_OPEN => Some(Self::Open),
            STATUS_APPLIED => Some(Self::Applied),
            STATUS_CLOSED => Some(Self::Closed),
            STATUS_DRAFT => Some(Self::Draft),
            _ => None,
        }
    }

    /// The word shown; an applied issue is "resolved", not "merged".
    fn label(self, target: &Event) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Applied if target.kind == Kind::from(ISSUE) => "resolved",
            Self::Applied => "merged",
            Self::Closed => "closed",
            Self::Draft => "draft",
        }
    }

    /// Whether the thread is still under the Open tab.
    fn is_open(self) -> bool {
        matches!(self, Self::Open | Self::Draft)
    }

    /// The stylesheet's colour class.
    fn class(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Applied => "done",
            Self::Closed => "closed",
            Self::Draft => "draft",
        }
    }

    fn icon(self, target: &Event) -> &'static str {
        let issue = target.kind == Kind::from(ISSUE);
        match (self, issue) {
            (Self::Open | Self::Draft, true) => icon::ISSUE,
            (Self::Applied, true) => icon::ISSUE_DONE,
            (Self::Closed, true) => icon::ISSUE_CLOSED,
            (Self::Open | Self::Draft, false) => icon::PULL,
            (Self::Applied, false) => icon::MERGED,
            (Self::Closed, false) => icon::PULL_CLOSED,
        }
    }

    /// The round badge a thread's page shows.
    fn badge(self, target: &Event) -> String {
        let label = self.label(target);
        let mut word = label.to_owned();
        if let Some(first) = word.get_mut(..1) {
            first.make_ascii_uppercase();
        }
        format!(
            "<span class=\"state {}\" data-status=\"{label}\">{}{word}</span>",
            self.class(),
            svg(self.icon(target), "icon-sm"),
        )
    }
}

/// The status of `target`: the newest of `statuses` that names it in an `e`
/// tag and is signed by an honoured author (ADR 0002, rule 3) — one of
/// `authorized` (the owner and the maintainers in the owner's announcement)
/// or the author of `target` itself. A `created_at` tie goes to the lowest
/// event id; with no honoured status the target is open. A pure function.
fn resolve_status(authorized: &[PublicKey], target: &Event, statuses: &[Event]) -> Status {
    let target_id = target.id.to_hex();
    newest(statuses.iter().filter(|status| {
        Status::from_kind(status.kind).is_some()
            && (status.pubkey == target.pubkey || authorized.contains(&status.pubkey))
            && status
                .tags
                .iter()
                .map(|tag| tag.as_slice())
                .any(|tag| matches!(tag, [name, id, ..] if name == "e" && *id == target_id))
    }))
    .and_then(|status| Status::from_kind(status.kind))
    .unwrap_or(Status::Open)
}

/// The NIP-34 status kinds a reader honours: open, applied, closed, draft.
pub const STATUS_KINDS: [u16; 4] = [STATUS_OPEN, STATUS_APPLIED, STATUS_CLOSED, STATUS_DRAFT];

/// Whether `target`'s honoured status is applied (a merged change, a
/// resolved issue): the newest of `statuses` that names it and comes from
/// `authorized` or from the target's own author (ADR 0002, rule 3). A later
/// reopen or close wins over an earlier resolve. A pure function.
pub fn is_applied(authorized: &[PublicKey], target: &Event, statuses: &[Event]) -> bool {
    resolve_status(authorized, target, statuses) == Status::Applied
}

fn coordinate(owner: &PublicKey, d: &str) -> String {
    format!("{ANNOUNCEMENT}:{}:{d}", owner.to_hex())
}

/// Whether `event` names the repo `coordinate` in an `a` tag.
fn belongs_to(event: &Event, coordinate: &str) -> bool {
    has_tag_value(event, "a", coordinate)
}

fn has_tag_value(event: &Event, name: &str, value: &str) -> bool {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .any(|tag| matches!(tag, [n, v, ..] if n == name && v == value))
}

/// A patch that starts a series: tagged `t: root`, or whose `e` tags name
/// none of `patches` (the repo's patch ids), whatever their marker.
fn is_root_patch(patch: &Event, patches: &[String]) -> bool {
    has_tag_value(patch, "t", "root")
        || !patch
            .tags
            .iter()
            .map(|tag| tag.as_slice())
            .any(|tag| matches!(tag, [name, id, ..] if name == "e" && patches.contains(id)))
}

/// The status of each of `targets`, from the statuses stored for them.
async fn statuses_of(
    store: &ReadOnlyStore,
    authorized: &[PublicKey],
    targets: &[&Event],
) -> Vec<Status> {
    if targets.is_empty() {
        return Vec::new();
    }
    // Only honoured keys are read, so a stranger's statuses cannot crowd an
    // honoured one out of the query's limit.
    let mut authors = authorized.to_vec();
    for target in targets {
        if !authors.contains(&target.pubkey) {
            authors.push(target.pubkey);
        }
    }
    let events = store
        .query(
            Filter::new()
                .kinds([
                    Kind::from(STATUS_OPEN),
                    Kind::from(STATUS_APPLIED),
                    Kind::from(STATUS_CLOSED),
                    Kind::from(STATUS_DRAFT),
                ])
                .events(targets.iter().map(|target| target.id))
                .authors(authors)
                .limit(LIMIT * 4),
        )
        .await;
    targets
        .iter()
        .map(|target| resolve_status(authorized, target, &events))
        .collect()
}

/// How many of the repo at `coordinate`'s issues, and of its patch series
/// and pull requests, are open (a draft counts as open), by their honoured
/// status. `authorized` is the repo's authorized authors (ADR 0002).
pub(crate) async fn open_counts(
    store: &ReadOnlyStore,
    coordinate: &str,
    authorized: &[PublicKey],
) -> (usize, usize) {
    let events: Vec<Event> = store
        .query(
            Filter::new()
                .kinds([
                    Kind::from(ISSUE),
                    Kind::from(PATCH),
                    Kind::from(PULL_REQUEST),
                ])
                .custom_tag(SingleLetterTag::LOWERCASE_A, coordinate.to_owned())
                .limit(LIMIT),
        )
        .await
        .into_iter()
        .filter(|event| belongs_to(event, coordinate))
        .collect();
    let patches: Vec<String> = events
        .iter()
        .filter(|event| event.kind == Kind::from(PATCH))
        .map(|event| event.id.to_hex())
        .collect();
    let issues: Vec<&Event> = events
        .iter()
        .filter(|event| event.kind == Kind::from(ISSUE))
        .collect();
    let pulls: Vec<&Event> = events
        .iter()
        .filter(|event| {
            event.kind == Kind::from(PULL_REQUEST)
                || (event.kind == Kind::from(PATCH) && is_root_patch(event, &patches))
        })
        .collect();
    let open = |statuses: Vec<Status>| statuses.into_iter().filter(|s| s.is_open()).count();
    (
        open(statuses_of(store, authorized, &issues).await),
        open(statuses_of(store, authorized, &pulls).await),
    )
}

/// Newest first, a tie to the lowest id: the order of the lists.
fn sort_newest_first(events: &mut [&Event]) {
    events.sort_by(|a, b| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
}

/// What a request for a thread page needs.
struct Repo {
    owner: PublicKey,
    d: String,
    announcement: Event,
    authorized: Vec<PublicKey>,
    /// The owner's display name, for the header.
    names: HashMap<PublicKey, String>,
    /// The apps the relay serves, for the header's links.
    apps: Apps,
}

impl Repo {
    async fn resolve(store: &ReadOnlyStore, apps: &Apps, npub: &str, d: &str) -> Option<Self> {
        let (owner, announcement) = resolve_repo(store, npub, d).await?;
        let authorized = authorized_authors(&announcement);
        let names = names(store, &[owner]).await;
        Some(Self {
            owner,
            d: d.to_owned(),
            announcement,
            authorized,
            names,
            apps: apps.clone(),
        })
    }

    fn base(&self) -> String {
        repo_path(&self.owner, &self.d)
    }

    fn coordinate(&self) -> String {
        coordinate(&self.owner, &self.d)
    }

    fn header(&self, tab: Tab) -> String {
        repo_header(
            &self.apps,
            &self.owner,
            &self.d,
            &self.announcement,
            &self.names,
            tab,
        )
    }

    /// The events of `kinds` that name this repo.
    async fn events(&self, store: &ReadOnlyStore, kinds: &[u16]) -> Vec<Event> {
        let coordinate = self.coordinate();
        store
            .query(
                Filter::new()
                    .kinds(kinds.iter().map(|kind| Kind::from(*kind)))
                    .custom_tag(SingleLetterTag::LOWERCASE_A, coordinate.clone())
                    .limit(LIMIT),
            )
            .await
            .into_iter()
            .filter(|event| belongs_to(event, &coordinate))
            .collect()
    }
}

/// The 64-char hex event id in a path, or none.
fn parse_id(id: &str) -> Option<EventId> {
    if id.len() == 64 {
        EventId::from_hex(id).ok()
    } else {
        None
    }
}

/// The title of a thread: its subject, else the start of its content.
fn title_of(event: &Event) -> String {
    let subject = first_tag(event, "subject").filter(|subject| !subject.trim().is_empty());
    let title = subject
        .or_else(|| event.content.lines().find(|line| !line.trim().is_empty()))
        .unwrap_or("(untitled)");
    title.chars().take(120).collect()
}

/// The labels of a thread: its `t` tags, but for the markers a patch
/// series uses.
fn labels(event: &Event) -> Vec<&str> {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter_map(|tag| match tag {
            [name, value, ..] if name == "t" => Some(value.as_str()),
            _ => None,
        })
        .filter(|label| !matches!(*label, "root" | "root-revision" | "revision-root"))
        .collect()
}

fn label_pills(event: &Event) -> String {
    labels(event)
        .iter()
        .map(|label| {
            format!(
                "<span class=\"pill pill-secondary\">{}</span>",
                escape(label)
            )
        })
        .collect()
}

/// Whether `?state=closed` asks for the closed tab.
fn wants_closed(query: Option<&str>) -> bool {
    query
        .into_iter()
        .flat_map(|query| query.split('&'))
        .any(|pair| pair == "state=closed")
}

#[allow(clippy::too_many_arguments)]
fn thread_list(
    repo: &Repo,
    tab: Tab,
    section: &str,
    noun: &str,
    threads: &[(&Event, Status)],
    names: &HashMap<PublicKey, String>,
    closed: bool,
) -> String {
    let mut body = repo.header(tab);
    let base = repo.base();
    let open_count = threads
        .iter()
        .filter(|(_, status)| status.is_open())
        .count();
    let closed_count = threads.len() - open_count;
    let (open_icon, closed_icon) = if section == "issues" {
        (icon::ISSUE, icon::ISSUE_DONE)
    } else {
        (icon::PULL, icon::MERGED)
    };
    body.push_str(&format!(
        "<div class=\"box\"><div class=\"box-header\"><nav class=\"state-tabs\">\
         <a class=\"{open_class}\" href=\"{base}/{section}\">{open_icon}{open_count} Open</a>\
         <a class=\"{closed_class}\" href=\"{base}/{section}?state=closed\">{closed_icon}{closed_count} Closed</a></nav></div>",
        open_class = if closed { "" } else { "current" },
        closed_class = if closed { "current" } else { "" },
        base = escape(&base),
        open_icon = svg(open_icon, ""),
        closed_icon = svg(closed_icon, ""),
    ));
    let shown: Vec<_> = threads
        .iter()
        .filter(|(_, status)| status.is_open() != closed)
        .collect();
    if shown.is_empty() {
        body.push_str(&format!(
            "<div class=\"empty\">No {} {noun}.</div>",
            if closed { "closed" } else { "open" }
        ));
    } else {
        body.push_str("<ul class=\"rows\">");
        for (event, status) in shown {
            let verb = if section == "issues" {
                "opened"
            } else {
                "proposed"
            };
            body.push_str(&format!(
                "<li class=\"thread-row\" data-status=\"{}\">{}<div>\
                 <a class=\"title\" href=\"{}/{section}/{}\">{}</a><span class=\"labels\">{}</span>\
                 <div class=\"meta\"><span class=\"mono\">{}</span><span>{verb} {} by {}</span>{}</div></div></li>",
                status.label(event),
                svg(status.icon(event), status.class()),
                escape(&base),
                event.id.to_hex(),
                escape(&title_of(event)),
                label_pills(event),
                &event.id.to_hex()[..7],
                ago(event.created_at),
                name(names, &event.pubkey),
                if *status == Status::Draft {
                    "<span class=\"pill\">Draft</span>"
                } else {
                    ""
                },
            ));
        }
        body.push_str("</ul>");
    }
    body.push_str("</div>");
    body
}

/// The authors of `events`, once each.
fn authors_of(events: &[&Event]) -> Vec<PublicKey> {
    let mut authors: Vec<PublicKey> = Vec::new();
    for event in events {
        if !authors.contains(&event.pubkey) {
            authors.push(event.pubkey);
        }
    }
    authors
}

pub(super) async fn issues(
    State(super::Pages { store, apps, .. }): State<super::Pages>,
    Path((npub, d)): Path<(String, String)>,
    RawQuery(query): RawQuery,
) -> Response {
    let Some(repo) = Repo::resolve(&store, &apps, &npub, &d).await else {
        return not_found(&apps);
    };
    let events = repo.events(&store, &[ISSUE]).await;
    let mut issues: Vec<&Event> = events
        .iter()
        .filter(|event| event.kind == Kind::from(ISSUE))
        .collect();
    sort_newest_first(&mut issues);
    let statuses = statuses_of(&store, &repo.authorized, &issues).await;
    let names = names(&store, &authors_of(&issues)).await;
    let threads: Vec<_> = issues.into_iter().zip(statuses).collect();
    let body = thread_list(
        &repo,
        Tab::Issues,
        "issues",
        "issues",
        &threads,
        &names,
        wants_closed(query.as_deref()),
    );
    html(&apps, StatusCode::OK, "Issues", &body)
}

pub(super) async fn pulls(
    State(super::Pages { store, apps, .. }): State<super::Pages>,
    Path((npub, d)): Path<(String, String)>,
    RawQuery(query): RawQuery,
) -> Response {
    let Some(repo) = Repo::resolve(&store, &apps, &npub, &d).await else {
        return not_found(&apps);
    };
    let events = repo.events(&store, &[PATCH, PULL_REQUEST]).await;
    let patches: Vec<String> = events
        .iter()
        .filter(|event| event.kind == Kind::from(PATCH))
        .map(|event| event.id.to_hex())
        .collect();
    let mut pulls: Vec<&Event> = events
        .iter()
        .filter(|event| {
            event.kind == Kind::from(PULL_REQUEST)
                || (event.kind == Kind::from(PATCH) && is_root_patch(event, &patches))
        })
        .collect();
    sort_newest_first(&mut pulls);
    let statuses = statuses_of(&store, &repo.authorized, &pulls).await;
    let names = names(&store, &authors_of(&pulls)).await;
    let threads: Vec<_> = pulls.into_iter().zip(statuses).collect();
    let body = thread_list(
        &repo,
        Tab::Pulls,
        "pulls",
        "patches or pull requests",
        &threads,
        &names,
        wants_closed(query.as_deref()),
    );
    html(&apps, StatusCode::OK, "Patches and pull requests", &body)
}

/// The target `id` if it is an event of one of `kinds` for `repo`.
async fn target(store: &ReadOnlyStore, repo: &Repo, id: &str, kinds: &[u16]) -> Option<Event> {
    let id = parse_id(id)?;
    let found = store.query(Filter::new().id(id)).await;
    let event = found.into_iter().find(|event| event.id == id)?;
    (kinds.contains(&event.kind.as_u16()) && belongs_to(&event, &repo.coordinate()))
        .then_some(event)
}

/// The comments on `target`: NIP-22 (`E`) and legacy kind 1622 (`e`), in order.
async fn comments(store: &ReadOnlyStore, target: &Event) -> Vec<Event> {
    let modern = store
        .query(
            Filter::new()
                .kind(Kind::from(COMMENT))
                .custom_tag(SingleLetterTag::UPPERCASE_E, target.id.to_hex())
                .limit(LIMIT),
        )
        .await;
    let legacy = store
        .query(
            Filter::new()
                .kind(Kind::from(LEGACY_COMMENT))
                .event(target.id)
                .limit(LIMIT),
        )
        .await;
    let id = target.id.to_hex();
    let mut found: Vec<Event> = modern
        .into_iter()
        .filter(|event| event.kind == Kind::from(COMMENT) && has_tag_value(event, "E", &id))
        .chain(legacy.into_iter().filter(|event| {
            event.kind == Kind::from(LEGACY_COMMENT) && has_tag_value(event, "e", &id)
        }))
        .collect();
    found.sort_by_key(|event| (event.created_at, event.id));
    found.dedup_by_key(|event| event.id);
    found
}

/// Markdown from someone else, as safe HTML.
fn markdown(text: &str) -> String {
    format!(
        "<div class=\"markdown small-text\">{}</div>",
        ui_markdown::to_html(&ui_markdown::parse(text, ui_markdown::absolute_only))
    )
}

/// A patch, its lines coloured as a diff's.
fn diff(text: &str) -> String {
    let mut out = String::from("<pre class=\"diff\">");
    for line in text.split_inclusive('\n') {
        // `-- ` above a patch's trailer is format-patch's signature line.
        let class = if line.starts_with("+++") || line.starts_with("---") || line.trim_end() == "--"
        {
            ""
        } else if line.starts_with('+') {
            "add"
        } else if line.starts_with('-') {
            "del"
        } else if line.starts_with("@@") {
            "hunk"
        } else {
            ""
        };
        out.push_str(&format!("<span class=\"{class}\">{}</span>", escape(line)));
    }
    out.push_str("</pre>");
    out
}

/// One comment: the avatar, then a card with who and when above the text.
fn comment(names: &HashMap<PublicKey, String>, event: &Event, verb: &str, text: &str) -> String {
    format!(
        "<div class=\"comment\">{}<div class=\"card\"><div class=\"card-head\">{}\
         <span class=\"muted\">{verb} {}</span></div>{text}</div></div>",
        avatar(&plain_name(names, &event.pubkey), false),
        name(names, &event.pubkey),
        ago(event.created_at),
    )
}

fn thread_body(
    repo: &Repo,
    target: &Event,
    status: Status,
    names: &HashMap<PublicKey, String>,
    side: &str,
    comments: &[Event],
) -> String {
    let (tab, noun, back) = match target.kind.as_u16() {
        ISSUE => (Tab::Issues, "issue", "issues"),
        PATCH => (Tab::Pulls, "patch", "pulls"),
        _ => (Tab::Pulls, "pull request", "pulls"),
    };
    let mut body = repo.header(tab);
    body.push_str(&format!(
        "<div class=\"thread-head\"><h1>{} <span class=\"num\">{}</span></h1>\
         <div class=\"by\">{}<span>{} opened this {noun} {} &middot; {} comment{}</span></div></div>",
        escape(&title_of(target)),
        &target.id.to_hex()[..7],
        status.badge(target),
        name(names, &target.pubkey),
        ago(target.created_at),
        comments.len(),
        if comments.len() == 1 { "" } else { "s" },
    ));
    body.push_str("<div class=\"split\"><div class=\"main\"><div class=\"comments\">");
    let opening = if target.kind == Kind::from(PATCH) {
        diff(&target.content)
    } else if target.content.trim().is_empty() {
        "<div class=\"card-body muted\"><em>No description provided.</em></div>".to_owned()
    } else {
        format!(
            "<div class=\"card-body\">{}</div>",
            markdown(&target.content)
        )
    };
    body.push_str(&comment(names, target, "opened", &opening));
    for reply in comments {
        body.push_str(&comment(
            names,
            reply,
            "commented",
            &format!(
                "<div class=\"card-body\">{}</div>",
                markdown(&reply.content)
            ),
        ));
    }
    body.push_str("</div>");
    if target.kind != Kind::from(ISSUE) {
        let (class, path, text) = match status {
            Status::Applied => (
                "done",
                icon::MERGED,
                "This change was applied and the thread closed.",
            ),
            Status::Closed => ("closed", icon::PULL_CLOSED, "Closed without being applied."),
            Status::Open | Status::Draft => (
                "open",
                icon::PULL,
                "Open. A maintainer applies it from their own machine, then publishes its status.",
            ),
        };
        body.push_str(&format!(
            "<div class=\"callout {class}\">{}<div class=\"text\">{text}</div></div>",
            svg(path, &format!("icon-lg {class}")),
        ));
    }
    body.push_str(&format!(
        "<p class=\"muted\"><a href=\"{}/{back}\">&larr; All {}</a></p></div>",
        escape(&repo.base()),
        if back == "issues" {
            "issues"
        } else {
            "patches and pull requests"
        }
    ));
    body.push_str("<aside class=\"sidebar narrow\"><div><h3>Labels</h3>");
    let pills = label_pills(target);
    if pills.is_empty() {
        body.push_str("<p class=\"muted small\">None yet</p>");
    } else {
        body.push_str(&format!("<div class=\"labels\">{pills}</div>"));
    }
    body.push_str("</div>");
    body.push_str(side);
    body.push_str("</aside></div>");
    body
}

pub(super) async fn issue(
    State(super::Pages { store, apps, .. }): State<super::Pages>,
    Path((npub, d, id)): Path<(String, String, String)>,
) -> Response {
    let Some(repo) = Repo::resolve(&store, &apps, &npub, &d).await else {
        return not_found(&apps);
    };
    let Some(issue) = target(&store, &repo, &id, &[ISSUE]).await else {
        return not_found(&apps);
    };
    let status = statuses_of(&store, &repo.authorized, &[&issue]).await[0];
    let comments = comments(&store, &issue).await;
    let people: Vec<PublicKey> = people(&issue, &comments);
    let names = names(&store, &people).await;
    let body = thread_body(&repo, &issue, status, &names, "", &comments);
    html(&apps, StatusCode::OK, &title_of(&issue), &body)
}

pub(super) async fn pull(
    State(super::Pages { store, apps, .. }): State<super::Pages>,
    Path((npub, d, id)): Path<(String, String, String)>,
) -> Response {
    let Some(repo) = Repo::resolve(&store, &apps, &npub, &d).await else {
        return not_found(&apps);
    };
    let Some(pull) = target(&store, &repo, &id, &[PATCH, PULL_REQUEST]).await else {
        return not_found(&apps);
    };
    let status = statuses_of(&store, &repo.authorized, &[&pull]).await[0];
    let comments = comments(&store, &pull).await;
    let people: Vec<PublicKey> = people(&pull, &comments);
    let names = names(&store, &people).await;

    let mut facts: Vec<(&str, String)> = Vec::new();
    if pull.kind == Kind::from(PULL_REQUEST) {
        for (heading, tag) in [
            ("Branch", "branch-name"),
            ("Clone", "clone"),
            ("Merge base", "merge-base"),
        ] {
            if let Some(value) = first_tag(&pull, tag) {
                facts.push((heading, value.to_owned()));
            }
        }
        if let Some(tip) = first_tag(&pull, "c") {
            facts.push(("Tip", tip.to_owned()));
        }
        // Only the PR's own author moves its tip.
        let updates = store
            .query(
                Filter::new()
                    .kind(Kind::from(PULL_REQUEST_UPDATE))
                    .custom_tag(SingleLetterTag::UPPERCASE_E, pull.id.to_hex())
                    .limit(LIMIT),
            )
            .await;
        let pull_id = pull.id.to_hex();
        let updates: Vec<&Event> = updates
            .iter()
            .filter(|update| {
                update.kind == Kind::from(PULL_REQUEST_UPDATE)
                    && update.pubkey == pull.pubkey
                    && has_tag_value(update, "E", &pull_id)
            })
            .collect();
        if let Some(latest) = newest(updates.iter().copied())
            && let Some(tip) = first_tag(latest, "c")
        {
            facts.push(("Latest tip", tip.to_owned()));
        }
        let mut ordered = updates.clone();
        ordered.sort_by_key(|update| (update.created_at, update.id));
        for (index, update) in ordered.iter().enumerate() {
            facts.push((
                if index == 0 { "Updates" } else { "" },
                first_tag(update, "c").unwrap_or("(no tip)").to_owned(),
            ));
        }
    }
    let mut extra = String::new();
    if !facts.is_empty() {
        extra.push_str("<div><h3>Details</h3><dl class=\"kv\">");
        for (heading, value) in facts {
            extra.push_str(&format!("<dt>{heading}</dt><dd>{}</dd>", escape(&value)));
        }
        extra.push_str("</dl></div>");
    }
    let body = thread_body(&repo, &pull, status, &names, &extra, &comments);
    html(&apps, StatusCode::OK, &title_of(&pull), &body)
}

/// The author of `target` and of each comment, once each.
fn people(target: &Event, comments: &[Event]) -> Vec<PublicKey> {
    let mut people = vec![target.pubkey];
    for comment in comments {
        if !people.contains(&comment.pubkey) {
            people.push(comment.pubkey);
        }
    }
    people
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;
    use nostr::types::Timestamp;

    use super::*;
    use crate::owner_announcement;

    fn tag(parts: &[&str]) -> Tag {
        Tag::parse(parts.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>()).unwrap()
    }

    fn sign(keys: &Keys, kind: u16, at: u64, tags: &[Tag]) -> Event {
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags.to_vec())
            .custom_created_at(Timestamp::from(at))
            .finalize(keys)
            .unwrap()
    }

    fn status(keys: &Keys, kind: u16, at: u64, target: &Event) -> Event {
        sign(keys, kind, at, &[tag(&["e", &target.id.to_hex()])])
    }

    struct Fixture {
        owner: Keys,
        maintainer: Keys,
        author: Keys,
        stranger: Keys,
        authorized: Vec<PublicKey>,
        issue: Event,
    }

    fn fixture() -> Fixture {
        let owner = Keys::generate();
        let maintainer = Keys::generate();
        let author = Keys::generate();
        let announcement = sign(
            &owner,
            ANNOUNCEMENT,
            1,
            &[
                tag(&["d", "r"]),
                tag(&["maintainers", &maintainer.public_key().to_hex()]),
            ],
        );
        let issue = sign(&author, ISSUE, 2, &[]);
        Fixture {
            authorized: authorized_authors(&announcement),
            owner,
            maintainer,
            author,
            stranger: Keys::generate(),
            issue,
        }
    }

    #[test]
    fn no_status_means_open() {
        let f = fixture();
        assert_eq!(resolve_status(&f.authorized, &f.issue, &[]), Status::Open);
    }

    #[test]
    fn owner_maintainer_and_author_are_honoured() {
        let f = fixture();
        for (keys, kind, want) in [
            (&f.owner, STATUS_CLOSED, Status::Closed),
            (&f.maintainer, STATUS_APPLIED, Status::Applied),
            (&f.author, STATUS_DRAFT, Status::Draft),
        ] {
            let events = [status(keys, kind, 10, &f.issue)];
            assert_eq!(resolve_status(&f.authorized, &f.issue, &events), want);
        }
    }

    #[test]
    fn an_issue_is_applied_only_while_its_honoured_status_says_so() {
        let f = fixture();
        let resolved = status(&f.maintainer, STATUS_APPLIED, 10, &f.issue);
        assert!(is_applied(
            &f.authorized,
            &f.issue,
            std::slice::from_ref(&resolved)
        ));
        let forged = status(&f.stranger, STATUS_APPLIED, 10, &f.issue);
        assert!(!is_applied(&f.authorized, &f.issue, &[forged]));
        let reopened = status(&f.owner, STATUS_OPEN, 20, &f.issue);
        assert!(!is_applied(&f.authorized, &f.issue, &[resolved, reopened]));
    }

    #[test]
    fn a_stranger_is_ignored_however_new() {
        let f = fixture();
        let events = [
            status(&f.owner, STATUS_CLOSED, 10, &f.issue),
            status(&f.stranger, STATUS_OPEN, 99, &f.issue),
        ];
        assert_eq!(
            resolve_status(&f.authorized, &f.issue, &events),
            Status::Closed
        );
    }

    #[test]
    fn the_newest_honoured_status_wins() {
        let f = fixture();
        let events = [
            status(&f.owner, STATUS_CLOSED, 10, &f.issue),
            status(&f.author, STATUS_OPEN, 20, &f.issue),
            status(&f.maintainer, STATUS_APPLIED, 15, &f.issue),
        ];
        assert_eq!(
            resolve_status(&f.authorized, &f.issue, &events),
            Status::Open
        );
    }

    #[test]
    fn a_tie_goes_to_the_lowest_event_id() {
        let f = fixture();
        let closed = status(&f.owner, STATUS_CLOSED, 10, &f.issue);
        let applied = status(&f.owner, STATUS_APPLIED, 10, &f.issue);
        let want = if closed.id < applied.id {
            Status::Closed
        } else {
            Status::Applied
        };
        for events in [[closed.clone(), applied.clone()], [applied, closed]] {
            assert_eq!(resolve_status(&f.authorized, &f.issue, &events), want);
        }
    }

    #[test]
    fn a_status_for_another_target_is_ignored() {
        let f = fixture();
        let other = sign(&f.author, ISSUE, 3, &[]);
        let events = [status(&f.owner, STATUS_CLOSED, 10, &other)];
        assert_eq!(
            resolve_status(&f.authorized, &f.issue, &events),
            Status::Open
        );
    }

    #[test]
    fn a_maintainer_only_in_a_non_owners_announcement_is_ignored() {
        let f = fixture();
        let impostor = Keys::generate();
        let rogue = sign(
            &impostor,
            ANNOUNCEMENT,
            5,
            &[
                tag(&["d", "r"]),
                tag(&["maintainers", &f.stranger.public_key().to_hex()]),
            ],
        );
        let own = sign(&f.owner, ANNOUNCEMENT, 1, &[tag(&["d", "r"])]);
        // The rogue announcement is newer, names the same `d`, and lists the
        // stranger; the owner's own announcement alone decides who is honoured.
        let announcements = [own, rogue];
        let announcement = owner_announcement(&f.owner.public_key(), "r", &announcements).unwrap();
        assert_eq!(announcement.pubkey, f.owner.public_key());
        let authorized = authorized_authors(announcement);
        let events = [status(&f.stranger, STATUS_CLOSED, 10, &f.issue)];
        assert_eq!(resolve_status(&authorized, &f.issue, &events), Status::Open);
    }

    #[test]
    fn root_patches_are_the_series_starts() {
        let keys = Keys::generate();
        let id = "a".repeat(64);
        let patches = [id.clone()];
        assert!(is_root_patch(&sign(&keys, PATCH, 1, &[]), &patches));
        assert!(is_root_patch(
            &sign(
                &keys,
                PATCH,
                1,
                &[tag(&["t", "root"]), tag(&["e", &id, "", "reply"])]
            ),
            &patches
        ));
        assert!(!is_root_patch(
            &sign(&keys, PATCH, 1, &[tag(&["e", &id, "", "reply"])]),
            &patches
        ));
        // An unmarked `e` to a patch is still a reply to it.
        assert!(!is_root_patch(
            &sign(&keys, PATCH, 1, &[tag(&["e", &id])]),
            &patches
        ));
        // Replying to something that is not a patch (an issue) is a root.
        let issue = "b".repeat(64);
        assert!(is_root_patch(
            &sign(&keys, PATCH, 1, &[tag(&["e", &issue, "", "reply"])]),
            &patches
        ));
    }

    #[test]
    fn only_a_64_hex_char_id_parses() {
        assert!(parse_id(&"a".repeat(64)).is_some());
        assert!(parse_id("abc").is_none());
        assert!(parse_id(&"g".repeat(64)).is_none());
    }
}
