//! The NIP-34 (git) app UI, mounted at `/repo` (#276).
//!
//! Server-rendered pages over this relay's repository events, with no
//! JavaScript: the list of repositories, a repository's home, and the
//! redirects from an `naddr` or an event id to the canonical
//! `/repo/<npub>/<d-tag>`, and a repository's issues and patches/PRs with their
//! threads and honoured status ([`threads`]). The list also loads the browser
//! half (`ui-nip34-web`, built by `cargo xtask ui-assets`) from
//! `/repo/assets/` when the relay has its assets. The tree and blob pages
//! (`git`) are layouts the browser half fills in: the relay never fetches a
//! git object.

mod git;
mod threads;

pub use git::{CONTENT_SECURITY_POLICY_VALUE as GIT_CONTENT_SECURITY_POLICY_VALUE, Defaults};

use std::collections::HashMap;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use nostr::event::{Event, EventId, Kind};
use nostr::filter::Filter;
use nostr::key::PublicKey;
use nostr::nips::nip19::{FromBech32, Nip19Coordinate, ToBech32};
use relay_ui::{AppUi, Assets, ReadOnlyStore};

/// The mount prefix.
pub const PREFIX: &str = "/repo";

/// NIP-34 repository announcement.
const ANNOUNCEMENT: u16 = 30_617;
/// NIP-34 repository state.
const STATE: u16 = 30_618;
/// The most announcements the list reads.
const LIST_LIMIT: usize = 500;

/// The manifest's name for the list page's script.
const SCRIPT: &str = "ui-nip34-web.js";

/// What the list says when there are no assets to browse with.
pub const UNAVAILABLE: &str = "git browsing unavailable";

/// What the pages read: the store, and the built assets if the relay has them.
#[derive(Clone)]
struct Pages {
    store: ReadOnlyStore,
    assets: Option<Assets>,
    defaults: Defaults,
}

/// The app UI over `store`, with the built `assets` if the relay has them and
/// the operator's `defaults` for where a browser looks for git objects.
pub fn app(store: ReadOnlyStore, assets: Option<Assets>, defaults: Defaults) -> AppUi {
    let router = Router::new()
        .route(PREFIX, get(list))
        .route(&format!("{PREFIX}/assets/{{name}}"), get(asset))
        .route("/repo/{target}", get(redirect))
        .route("/repo/{npub}/{d}", get(home))
        .route("/repo/{npub}/{d}/tree/{*rest}", get(git::tree))
        .route("/repo/{npub}/{d}/blob/{*rest}", get(git::blob))
        .route("/repo/{npub}/{d}/issues", get(threads::issues))
        .route("/repo/{npub}/{d}/issues/{id}", get(threads::issue))
        .route("/repo/{npub}/{d}/pulls", get(threads::pulls))
        .route("/repo/{npub}/{d}/pulls/{id}", get(threads::pull))
        .fallback(not_found)
        .with_state(Pages {
            store,
            assets,
            defaults,
        });
    AppUi::new(PREFIX, router)
}

async fn asset(State(pages): State<Pages>, Path(name): Path<String>) -> Response {
    match pages.assets {
        Some(assets) => assets.respond(&name),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

/// The values of the first tag called `name`, after the name.
fn tag_values<'a>(event: &'a Event, name: &str) -> &'a [String] {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find(|tag| tag.first().is_some_and(|first| first == name))
        .map_or(&[], |tag| &tag[1..])
}

fn first_tag<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    tag_values(event, name).first().map(String::as_str)
}

fn d_tag(event: &Event) -> &str {
    first_tag(event, "d").unwrap_or("")
}

/// Whether `candidate` is a newer replacement than `held`: later, and on a
/// `created_at` tie the lowest event id (ADR 0002).
fn supersedes(candidate: &Event, held: &Event) -> bool {
    (candidate.created_at, held.id) > (held.created_at, candidate.id)
}

/// The newest of `events`, or none.
fn newest<'a>(events: impl IntoIterator<Item = &'a Event>) -> Option<&'a Event> {
    events
        .into_iter()
        .fold(None, |held: Option<&Event>, candidate| match held {
            Some(held) if !supersedes(candidate, held) => Some(held),
            _ => Some(candidate),
        })
}

/// The pubkeys whose repo state is honoured: the owner and the maintainers
/// in the owner's own announcement (ADR 0002). A pure function.
fn authorized_authors(announcement: &Event) -> Vec<PublicKey> {
    let mut authors = vec![announcement.pubkey];
    for value in tag_values(announcement, "maintainers") {
        if let Ok(key) = PublicKey::from_hex(value)
            && !authors.contains(&key)
        {
            authors.push(key);
        }
    }
    authors
}

/// The honoured repo state among `states` for `announcement`: the newest
/// signed by an authorized author (ADR 0002). A pure function.
fn honoured_state<'a>(announcement: &Event, states: &'a [Event]) -> Option<&'a Event> {
    let authors = authorized_authors(announcement);
    let d = d_tag(announcement);
    newest(
        states
            .iter()
            .filter(|state| state.kind == Kind::from(STATE))
            .filter(|state| authors.contains(&state.pubkey) && d_tag(state) == d),
    )
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// A link target is only followed if it is `http(s)`.
fn link(url: &str) -> String {
    if url.starts_with("https://") || url.starts_with("http://") {
        format!("<a href=\"{0}\" rel=\"nofollow\">{0}</a>", escape(url))
    } else {
        format!("<code>{}</code>", escape(url))
    }
}

fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn npub(key: &PublicKey) -> String {
    let Ok(npub) = key.to_bech32();
    npub
}

fn repo_path(owner: &PublicKey, d: &str) -> String {
    format!("{PREFIX}/{}/{}", npub(owner), percent_encode(d))
}

fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>{}</title></head><body>{body}</body></html>",
        escape(title)
    )
}

fn html(status: StatusCode, title: &str, body: &str) -> Response {
    (status, Html(page(title, body))).into_response()
}

async fn not_found() -> Response {
    html(
        StatusCode::NOT_FOUND,
        "Not found",
        "<h1>Not found</h1><p><a href=\"/repo\">Repositories</a></p>",
    )
}

/// Display names from kind 0 profiles, by pubkey.
async fn names(store: &ReadOnlyStore, keys: &[PublicKey]) -> HashMap<PublicKey, String> {
    if keys.is_empty() {
        return HashMap::new();
    }
    let profiles = store
        .query(Filter::new().kind(Kind::Metadata).authors(keys.to_vec()))
        .await;
    let mut names = HashMap::new();
    for key in keys {
        let Some(profile) = newest(profiles.iter().filter(|event| event.pubkey == *key)) else {
            continue;
        };
        let Ok(serde_json::Value::Object(fields)) = serde_json::from_str(&profile.content) else {
            continue;
        };
        let name = ["display_name", "name"]
            .iter()
            .filter_map(|field| fields.get(*field).and_then(|value| value.as_str()))
            .find(|name| !name.trim().is_empty());
        if let Some(name) = name {
            names.insert(*key, name.trim().to_owned());
        }
    }
    names
}

/// A person: the profile's name where there is one, else the npub.
fn person(names: &HashMap<PublicKey, String>, key: &PublicKey) -> String {
    let npub = npub(key);
    match names.get(key) {
        Some(name) => format!("{} (<code>{}</code>)", escape(name), escape(&npub)),
        None => format!("<code>{}</code>", escape(&npub)),
    }
}

/// The newest announcement for each `(pubkey, d)`.
fn latest_announcements(events: &[Event]) -> Vec<&Event> {
    let mut by_address: HashMap<(PublicKey, &str), &Event> = HashMap::new();
    for event in events.iter().filter(|e| e.kind == Kind::from(ANNOUNCEMENT)) {
        let held = by_address
            .entry((event.pubkey, d_tag(event)))
            .or_insert(event);
        if supersedes(event, held) {
            *held = event;
        }
    }
    let mut latest: Vec<&Event> = by_address.into_values().collect();
    latest.sort_by(|a, b| (d_tag(a), a.pubkey.to_hex()).cmp(&(d_tag(b), b.pubkey.to_hex())));
    latest
}

fn repo_name(announcement: &Event) -> &str {
    first_tag(announcement, "name")
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| d_tag(announcement))
}

async fn list(State(Pages { store, assets, .. }): State<Pages>) -> Response {
    let events = store
        .query(
            Filter::new()
                .kind(Kind::from(ANNOUNCEMENT))
                .limit(LIST_LIMIT),
        )
        .await;
    let repos = latest_announcements(&events);
    let owners: Vec<PublicKey> = repos.iter().map(|repo| repo.pubkey).collect();
    let names = names(&store, &owners).await;
    let mut body = String::from("<h1>Repositories</h1>");
    if repos.is_empty() {
        body.push_str("<p>No repositories on this relay.</p>");
    } else {
        body.push_str("<ul>");
        for repo in repos {
            body.push_str(&format!(
                "<li><a href=\"{}\">{}</a> by {}",
                escape(&repo_path(&repo.pubkey, d_tag(repo))),
                escape(repo_name(repo)),
                person(&names, &repo.pubkey),
            ));
            if let Some(description) = first_tag(repo, "description") {
                body.push_str(&format!(" &mdash; {}", escape(description)));
            }
            body.push_str("</li>");
        }
        body.push_str("</ul>");
    }
    match assets.and_then(|assets| assets.url(PREFIX, SCRIPT)) {
        Some(script) => body.push_str(&format!(
            "<p id=\"app\"></p><script type=\"module\" src=\"{}\"></script>",
            escape(&script)
        )),
        None => body.push_str(&format!("<p>{UNAVAILABLE}</p>")),
    }
    html(StatusCode::OK, "Repositories", &body)
}

/// The repo at `/repo/<npub>/<d>`: its owner and the owner's latest
/// announcement, or none.
async fn resolve_repo(store: &ReadOnlyStore, npub: &str, d: &str) -> Option<(PublicKey, Event)> {
    let owner = PublicKey::from_bech32(npub).ok()?;
    let announcements = store
        .query(
            Filter::new()
                .kind(Kind::from(ANNOUNCEMENT))
                .author(owner)
                .identifier(d),
        )
        .await;
    let announcement = owner_announcement(&owner, d, &announcements)?;
    Some((owner, announcement.clone()))
}

/// The repo's announcement among `announcements`: the newest `owner` signed
/// for `d`. Another key's announcement for the same `d` never stands in for
/// it, so its `maintainers` are never honoured (ADR 0002). A pure function.
fn owner_announcement<'a>(
    owner: &PublicKey,
    d: &str,
    announcements: &'a [Event],
) -> Option<&'a Event> {
    newest(
        announcements
            .iter()
            .filter(|event| event.kind == Kind::from(ANNOUNCEMENT))
            .filter(|event| event.pubkey == *owner && d_tag(event) == d),
    )
}

async fn home(
    State(Pages { store, .. }): State<Pages>,
    Path((owner, d)): Path<(String, String)>,
) -> Response {
    let Some((owner, announcement)) = resolve_repo(&store, &owner, &d).await else {
        return not_found().await;
    };
    let announcement = &announcement;
    let authors = authorized_authors(announcement);
    let states = store
        .query(
            Filter::new()
                .kind(Kind::from(STATE))
                .authors(authors.clone())
                .identifier(d.clone()),
        )
        .await;
    let state = honoured_state(announcement, &states);
    let names = names(&store, &authors).await;

    let mut body = format!("<h1>{}</h1>", escape(repo_name(announcement)));
    if let Some(description) = first_tag(announcement, "description") {
        body.push_str(&format!("<p>{}</p>", escape(description)));
    }
    body.push_str(&format!(
        "<p>Owner: {}</p><h2>Maintainers</h2><ul>",
        person(&names, &owner)
    ));
    for key in authors.iter().filter(|key| **key != owner) {
        body.push_str(&format!("<li>{}</li>", person(&names, key)));
    }
    body.push_str("</ul>");
    for (heading, tag) in [("Clone", "clone"), ("Web", "web")] {
        let urls = tag_values(announcement, tag);
        if !urls.is_empty() {
            body.push_str(&format!("<h2>{heading}</h2><ul>"));
            for url in urls {
                body.push_str(&format!("<li>{}</li>", link(url)));
            }
            body.push_str("</ul>");
        }
    }
    body.push_str("<h2>Refs</h2>");
    match state {
        Some(state) => {
            if let Some(head) = first_tag(state, "HEAD") {
                let branch = head.strip_prefix("ref: ").unwrap_or(head);
                let branch = branch.strip_prefix("refs/heads/").unwrap_or(branch);
                body.push_str(&format!(
                    "<p>Default branch: <code>{}</code></p>",
                    escape(branch)
                ));
                body.push_str(&format!(
                    "<p><a href=\"{}/tree/{}\">Browse files</a></p>",
                    escape(&repo_path(&owner, &d)),
                    escape(
                        &branch
                            .split('/')
                            .map(percent_encode)
                            .collect::<Vec<_>>()
                            .join("/")
                    )
                ));
            }
            body.push_str("<ul>");
            for tag in state.tags.iter().map(|tag| tag.as_slice()) {
                if let [name, commit, ..] = tag
                    && name.starts_with("refs/")
                {
                    body.push_str(&format!(
                        "<li><code>{}</code> <code>{}</code></li>",
                        escape(name),
                        escape(commit)
                    ));
                }
            }
            body.push_str("</ul>");
        }
        None => body.push_str("<p>No refs published.</p>"),
    }
    let base = repo_path(&owner, &d);
    body.push_str(&format!(
        "<p><a href=\"{0}/issues\">Issues</a> &middot; <a href=\"{0}/pulls\">Patches and pull requests</a></p>",
        escape(&base)
    ));
    body.push_str("<p><a href=\"/repo\">All repositories</a></p>");
    html(StatusCode::OK, repo_name(announcement), &body)
}

fn moved(path: &str) -> Response {
    (StatusCode::MOVED_PERMANENTLY, [(header::LOCATION, path)]).into_response()
}

/// `/repo/<naddr>` and `/repo/<event-id>`: a redirect to the repo's canonical
/// path.
async fn redirect(
    State(Pages { store, .. }): State<Pages>,
    Path(target): Path<String>,
) -> Response {
    if let Ok(address) = Nip19Coordinate::from_bech32(&target) {
        if address.kind == Kind::from(ANNOUNCEMENT) {
            return moved(&repo_path(&address.public_key, &address.identifier));
        }
        return not_found().await;
    }
    let id = EventId::from_hex(&target).ok().or_else(|| {
        // `nevent` and `note` carry an id too.
        match nostr::nips::nip19::Nip19::from_bech32(&target).ok()? {
            nostr::nips::nip19::Nip19::EventId(id) => Some(id),
            nostr::nips::nip19::Nip19::Event(event) => Some(event.event_id),
            _ => None,
        }
    });
    let Some(id) = id else {
        return not_found().await;
    };
    let events = store.query(Filter::new().id(id)).await;
    let Some(event) = events.iter().find(|event| event.id == id) else {
        return not_found().await;
    };
    if event.kind == Kind::from(ANNOUNCEMENT) {
        return moved(&repo_path(&event.pubkey, d_tag(event)));
    }
    // A repo-scoped event names its repo in an `a` tag.
    for tag in event.tags.iter().map(|tag| tag.as_slice()) {
        let [name, address, ..] = tag else { continue };
        if name != "a" {
            continue;
        }
        let mut parts = address.splitn(3, ':');
        if let (Some(kind), Some(owner), Some(d)) = (parts.next(), parts.next(), parts.next())
            && kind.parse() == Ok(ANNOUNCEMENT)
            && let Ok(owner) = PublicKey::from_hex(owner)
        {
            return moved(&repo_path(&owner, d));
        }
    }
    not_found().await
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;
    use nostr::types::Timestamp;

    use super::*;

    fn tag(parts: &[&str]) -> Tag {
        Tag::parse(
            parts
                .iter()
                .map(|part| (*part).to_owned())
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    fn sign(keys: &Keys, kind: u16, at: u64, tags: &[Tag]) -> Event {
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags.to_vec())
            .custom_created_at(Timestamp::from(at))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn only_authorized_authors_set_the_refs() {
        let owner = Keys::generate();
        let maintainer = Keys::generate();
        let stranger = Keys::generate();
        let announcement = sign(
            &owner,
            ANNOUNCEMENT,
            1,
            &[
                tag(&["d", "r"]),
                tag(&["maintainers", &maintainer.public_key().to_hex()]),
            ],
        );
        let state = |keys: &Keys, at| sign(keys, STATE, at, &[tag(&["d", "r"])]);
        let owners = state(&owner, 10);
        let maintainers = state(&maintainer, 20);
        let strangers = state(&stranger, 30);
        let states = [owners.clone(), maintainers.clone(), strangers];
        assert_eq!(
            honoured_state(&announcement, &states).unwrap().id,
            maintainers.id
        );
        assert_eq!(
            honoured_state(&announcement, &states[..1]).unwrap().id,
            owners.id
        );
        assert!(honoured_state(&announcement, &states[2..]).is_none());
    }

    #[test]
    fn a_tie_goes_to_the_lowest_event_id() {
        let owner = Keys::generate();
        let announcement = sign(&owner, ANNOUNCEMENT, 1, &[tag(&["d", "r"])]);
        let a = sign(&owner, STATE, 5, &[tag(&["d", "r"]), tag(&["x", "1"])]);
        let b = sign(&owner, STATE, 5, &[tag(&["d", "r"]), tag(&["x", "2"])]);
        let lowest = if a.id < b.id { a.id } else { b.id };
        for states in [[a.clone(), b.clone()], [b, a]] {
            assert_eq!(honoured_state(&announcement, &states).unwrap().id, lowest);
        }
    }

    #[test]
    fn markup_in_a_tag_is_escaped() {
        assert_eq!(escape("<script>\"&'"), "&lt;script&gt;&quot;&amp;&#39;");
    }
}
