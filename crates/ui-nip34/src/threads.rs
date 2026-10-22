//! A repository's issues and patches/PRs: lists, threads and honoured status.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use nostr::event::{Event, EventId, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use relay_ui::ReadOnlyStore;

use super::{
    ANNOUNCEMENT, authorized_authors, escape, first_tag, html, names, newest, not_found, person,
    repo_name, repo_path, resolve_repo,
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
            Self::Applied => "applied/merged",
            Self::Closed => "closed",
            Self::Draft => "draft",
        }
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

fn coordinate(owner: &PublicKey, d: &str) -> String {
    format!("{ANNOUNCEMENT}:{}:{d}", owner.to_hex())
}

/// Whether `event` names the repo `coordinate` in an `a` tag.
fn belongs_to(event: &Event, coordinate: &str) -> bool {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .any(|tag| matches!(tag, [name, value, ..] if name == "a" && value == coordinate))
}

fn has_tag_value(event: &Event, name: &str, value: &str) -> bool {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .any(|tag| matches!(tag, [n, v, ..] if n == name && v == value))
}

/// A patch that starts a series: tagged `t: root`, or replying to no patch.
fn is_root_patch(patch: &Event) -> bool {
    has_tag_value(patch, "t", "root")
        || !patch
            .tags
            .iter()
            .map(|tag| tag.as_slice())
            .any(|tag| matches!(tag, [name, _, _, marker, ..] if name == "e" && marker == "reply"))
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
                .limit(LIMIT * 4),
        )
        .await;
    targets
        .iter()
        .map(|target| resolve_status(authorized, target, &events))
        .collect()
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
}

impl Repo {
    async fn resolve(store: &ReadOnlyStore, npub: &str, d: &str) -> Option<Self> {
        let (owner, announcement) = resolve_repo(store, npub, d).await?;
        let authorized = authorized_authors(&announcement);
        Some(Self {
            owner,
            d: d.to_owned(),
            announcement,
            authorized,
        })
    }

    fn base(&self) -> String {
        repo_path(&self.owner, &self.d)
    }

    fn coordinate(&self) -> String {
        coordinate(&self.owner, &self.d)
    }

    fn header(&self, title: &str) -> String {
        format!(
            "<p><a href=\"{}\">{}</a></p><h1>{}</h1>",
            escape(&self.base()),
            escape(repo_name(&self.announcement)),
            escape(title)
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

fn thread_list(
    heading: &str,
    repo: &Repo,
    section: &str,
    threads: &[(&Event, Status)],
    empty: &str,
) -> String {
    let mut body = repo.header(heading);
    if threads.is_empty() {
        body.push_str(&format!("<p>{}</p>", escape(empty)));
    } else {
        body.push_str("<ul>");
        for (event, status) in threads {
            body.push_str(&format!(
                "<li><a href=\"{}/{section}/{}\">{}</a> [{}]</li>",
                escape(&repo.base()),
                event.id.to_hex(),
                escape(&title_of(event)),
                status.label(event)
            ));
        }
        body.push_str("</ul>");
    }
    body
}

pub(super) async fn issues(
    State(store): State<ReadOnlyStore>,
    Path((npub, d)): Path<(String, String)>,
) -> Response {
    let Some(repo) = Repo::resolve(&store, &npub, &d).await else {
        return not_found().await;
    };
    let events = repo.events(&store, &[ISSUE]).await;
    let mut issues: Vec<&Event> = events
        .iter()
        .filter(|event| event.kind == Kind::from(ISSUE))
        .collect();
    sort_newest_first(&mut issues);
    let statuses = statuses_of(&store, &repo.authorized, &issues).await;
    let threads: Vec<_> = issues.into_iter().zip(statuses).collect();
    let body = thread_list("Issues", &repo, "issues", &threads, "No issues.");
    html(StatusCode::OK, "Issues", &body)
}

pub(super) async fn pulls(
    State(store): State<ReadOnlyStore>,
    Path((npub, d)): Path<(String, String)>,
) -> Response {
    let Some(repo) = Repo::resolve(&store, &npub, &d).await else {
        return not_found().await;
    };
    let events = repo.events(&store, &[PATCH, PULL_REQUEST]).await;
    let mut pulls: Vec<&Event> = events
        .iter()
        .filter(|event| {
            event.kind == Kind::from(PULL_REQUEST)
                || (event.kind == Kind::from(PATCH) && is_root_patch(event))
        })
        .collect();
    sort_newest_first(&mut pulls);
    let statuses = statuses_of(&store, &repo.authorized, &pulls).await;
    let threads: Vec<_> = pulls.into_iter().zip(statuses).collect();
    let body = thread_list(
        "Patches and pull requests",
        &repo,
        "pulls",
        &threads,
        "No patches or pull requests.",
    );
    html(StatusCode::OK, "Patches and pull requests", &body)
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

fn thread_body(
    repo: &Repo,
    target: &Event,
    status: Status,
    names: &std::collections::HashMap<PublicKey, String>,
    extra: &str,
    comments: &[Event],
) -> String {
    let (content_label, back) = match target.kind.as_u16() {
        ISSUE => ("Description", ("issues", "All issues")),
        PATCH => ("Patch", ("pulls", "All patches and pull requests")),
        _ => ("Description", ("pulls", "All patches and pull requests")),
    };
    let mut body = repo.header(&title_of(target));
    body.push_str(&format!(
        "<p>Status: <strong>{}</strong></p><p>By {}</p>{extra}\
         <h2>{}</h2><pre>{}</pre><h2>Comments</h2>",
        status.label(target),
        person(names, &target.pubkey),
        escape(content_label),
        escape(&target.content)
    ));
    if comments.is_empty() {
        body.push_str("<p>No comments.</p>");
    } else {
        for comment in comments {
            body.push_str(&format!(
                "<div><p>{}</p><pre>{}</pre></div>",
                person(names, &comment.pubkey),
                escape(&comment.content)
            ));
        }
    }
    body.push_str(&format!(
        "<p><a href=\"{}/{}\">{}</a></p>",
        escape(&repo.base()),
        back.0,
        escape(back.1)
    ));
    body
}

pub(super) async fn issue(
    State(store): State<ReadOnlyStore>,
    Path((npub, d, id)): Path<(String, String, String)>,
) -> Response {
    let Some(repo) = Repo::resolve(&store, &npub, &d).await else {
        return not_found().await;
    };
    let Some(issue) = target(&store, &repo, &id, &[ISSUE]).await else {
        return not_found().await;
    };
    let status = statuses_of(&store, &repo.authorized, &[&issue]).await[0];
    let comments = comments(&store, &issue).await;
    let people: Vec<PublicKey> = people(&issue, &comments);
    let names = names(&store, &people).await;
    let body = thread_body(&repo, &issue, status, &names, "", &comments);
    html(StatusCode::OK, &title_of(&issue), &body)
}

pub(super) async fn pull(
    State(store): State<ReadOnlyStore>,
    Path((npub, d, id)): Path<(String, String, String)>,
) -> Response {
    let Some(repo) = Repo::resolve(&store, &npub, &d).await else {
        return not_found().await;
    };
    let Some(pull) = target(&store, &repo, &id, &[PATCH, PULL_REQUEST]).await else {
        return not_found().await;
    };
    let status = statuses_of(&store, &repo.authorized, &[&pull]).await[0];
    let comments = comments(&store, &pull).await;
    let people: Vec<PublicKey> = people(&pull, &comments);
    let names = names(&store, &people).await;

    let mut extra = String::new();
    if pull.kind == Kind::from(PULL_REQUEST) {
        for (heading, tag) in [("Clone", "clone"), ("Merge base", "merge-base")] {
            if let Some(value) = first_tag(&pull, tag) {
                extra.push_str(&format!("<p>{heading}: <code>{}</code></p>", escape(value)));
            }
        }
        if let Some(tip) = first_tag(&pull, "c") {
            extra.push_str(&format!("<p>Tip: <code>{}</code></p>", escape(tip)));
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
            extra.push_str(&format!("<p>Latest tip: <code>{}</code></p>", escape(tip)));
        }
        if !updates.is_empty() {
            let mut ordered = updates.clone();
            ordered.sort_by_key(|update| (update.created_at, update.id));
            extra.push_str("<h2>Updates</h2><ul>");
            for update in ordered {
                extra.push_str(&format!(
                    "<li><code>{}</code></li>",
                    escape(first_tag(update, "c").unwrap_or("(no tip)"))
                ));
            }
            extra.push_str("</ul>");
        }
    }
    let body = thread_body(&repo, &pull, status, &names, &extra, &comments);
    html(StatusCode::OK, &title_of(&pull), &body)
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
        // The owner's announcement alone decides who is honoured.
        let authorized = authorized_authors(&sign(&f.owner, ANNOUNCEMENT, 1, &[tag(&["d", "r"])]));
        assert!(authorized_authors(&rogue).contains(&f.stranger.public_key()));
        let events = [status(&f.stranger, STATUS_CLOSED, 10, &f.issue)];
        assert_eq!(resolve_status(&authorized, &f.issue, &events), Status::Open);
    }

    #[test]
    fn root_patches_are_the_series_starts() {
        let keys = Keys::generate();
        let id = "a".repeat(64);
        assert!(is_root_patch(&sign(&keys, PATCH, 1, &[])));
        assert!(is_root_patch(&sign(
            &keys,
            PATCH,
            1,
            &[tag(&["t", "root"]), tag(&["e", &id, "", "reply"])]
        )));
        assert!(!is_root_patch(&sign(
            &keys,
            PATCH,
            1,
            &[tag(&["e", &id, "", "reply"])]
        )));
    }

    #[test]
    fn only_a_64_hex_char_id_parses() {
        assert!(parse_id(&"a".repeat(64)).is_some());
        assert!(parse_id("abc").is_none());
        assert!(parse_id(&"g".repeat(64)).is_none());
    }
}
