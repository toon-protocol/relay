//! The NIP-34 (git) app UI, mounted at `/repo` (#276), in rig-web's look
//! (`markup`, and `relay_ui`'s stylesheet).
//!
//! Server-rendered pages over this relay's repository events, with no
//! JavaScript: the list of repositories, the redirects from an `naddr` or an
//! event id to the canonical `/repo/<npub>/<d-tag>`, and a repository's
//! issues and patches/PRs with their threads and honoured status
//! ([`threads`]). The list also loads the browser half (`ui-nip34-web`, built
//! by `cargo xtask ui-assets`) from `/repo/assets/` when the relay has its
//! assets. A repository's home, like rig-web's, lists its files and README
//! beside its About sidebar, so it is a git page too: the home, tree, blob
//! and commit pages (`git`) are layouts the browser half fills in, and the
//! relay never fetches a git object. A blob page also loads the highlighter
//! (`ui-highlight-web`, #287), its own module, which no other page names.

mod git;
mod markup;
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

use markup::{Tab, ago, avatar, icon, name, plain_name, ref_menu, repo_header, svg};

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
/// The manifest's name for the blob page's highlighter (#287).
const HIGHLIGHT_SCRIPT: &str = "ui-highlight-web.js";

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
        .route("/repo/{npub}/{d}/commits", get(git::commits_default))
        .route("/repo/{npub}/{d}/commits/{*rest}", get(git::commits))
        .route("/repo/{npub}/{d}/commit/{id}", get(git::commit))
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
    if name == relay_ui::stylesheet_name() {
        return relay_ui::stylesheet_response();
    }
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

/// The refs a repo state names, `(name, commit)`, in both shapes a state is
/// written in (rig's rule, rig#156): NIP-34's `["refs/heads/main", <sha>]`,
/// and the legacy `["r", "refs/heads/main", <sha>]` that every repo pushed by
/// the rig CLI carries. When both name a ref, NIP-34's wins. A peeled tag
/// (`refs/tags/v1^{}`) is not a ref. A pure function.
pub(crate) fn refs(state: &Event) -> Vec<(&str, &str)> {
    let is_ref = |name: &str| {
        (name.starts_with("refs/heads/") || name.starts_with("refs/tags/"))
            && !name.ends_with("^{}")
    };
    let mut found: Vec<(&str, &str)> = Vec::new();
    for tag in state.tags.iter().map(|tag| tag.as_slice()) {
        if let [name, commit, ..] = tag
            && is_ref(name)
            && !found.iter().any(|(held, _)| held == name)
        {
            found.push((name, commit));
        }
    }
    for tag in state.tags.iter().map(|tag| tag.as_slice()) {
        if let [marker, name, commit, ..] = tag
            && marker == "r"
            && is_ref(name)
            && !found.iter().any(|(held, _)| held == name)
        {
            found.push((name, commit));
        }
    }
    found
}

/// The branch a repo state's `HEAD` names: NIP-34's `["HEAD", "ref:
/// refs/heads/main"]`, else rig's `["r", "HEAD", …]`, else `main` as rig-web
/// assumed. A pure function.
pub(crate) fn head_branch(state: &Event) -> String {
    let nip = first_tag(state, "HEAD");
    let legacy = state
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find_map(|tag| match tag {
            [marker, head, value, ..] if marker == "r" && head == "HEAD" => Some(value.as_str()),
            _ => None,
        });
    nip.or(legacy)
        .map(|head| {
            let head = head.strip_prefix("ref:").unwrap_or(head).trim();
            head.strip_prefix("refs/heads/").unwrap_or(head).to_owned()
        })
        .filter(|head| !head.is_empty())
        .unwrap_or_else(|| "main".to_owned())
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
        format!(
            "<a class=\"truncate\" href=\"{0}\" rel=\"nofollow\">{0}</a>",
            escape(url)
        )
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

fn html(status: StatusCode, title: &str, body: &str) -> Response {
    (status, Html(markup::page(title, body))).into_response()
}

async fn not_found() -> Response {
    html(
        StatusCode::NOT_FOUND,
        "Not found",
        "<div class=\"box\"><div class=\"empty\"><h2>Not found</h2>\
         <p>Nothing on this relay lives at this address.</p>\
         <p><a href=\"/repo\">All repositories</a></p></div></div>",
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
    let mut body = String::from("<h1 class=\"page-title\">Repositories</h1>");
    if repos.is_empty() {
        body.push_str(
            "<div class=\"box\"><div class=\"empty\">No repositories on this relay.</div></div>",
        );
    } else {
        body.push_str(&format!(
            "<div class=\"box\"><div class=\"box-header strong\">{} {}</div><ul class=\"rows\">",
            repos.len(),
            if repos.len() == 1 {
                "repository"
            } else {
                "repositories"
            }
        ));
        for repo in repos {
            body.push_str(&format!(
                "<li class=\"repo-row\"><div class=\"title\"><a href=\"{}\">{}</a>\
                 <span class=\"pill\">Public</span></div>",
                escape(&repo_path(&repo.pubkey, d_tag(repo))),
                escape(repo_name(repo)),
            ));
            if let Some(description) = first_tag(repo, "description")
                .map(str::trim)
                .filter(|text| !text.is_empty())
            {
                body.push_str(&format!("<p>{}</p>", escape(description)));
            }
            body.push_str(&format!(
                "<div class=\"meta\">{}<span>Updated {}</span></div></li>",
                name(&names, &repo.pubkey),
                ago(repo.created_at),
            ));
        }
        body.push_str("</ul></div>");
    }
    match assets.and_then(|assets| assets.url(PREFIX, SCRIPT)) {
        Some(script) => body.push_str(&format!(
            "<script type=\"module\" src=\"{}\"></script>",
            escape(&script)
        )),
        None => body.push_str(&format!("<p class=\"muted small\">{UNAVAILABLE}</p>")),
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
    State(Pages {
        store,
        assets,
        defaults,
    }): State<Pages>,
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
    let base = repo_path(&owner, &d);

    let mut body = repo_header(&owner, &d, announcement, &names, Tab::Code);
    body.push_str("<div class=\"split\"><div class=\"main\"><div class=\"toolbar\">");
    let default_branch = state.map(head_branch);
    match (state, &default_branch) {
        (Some(state), Some(branch)) => body.push_str(&ref_menu(&base, "tree", branch, state)),
        _ => body.push_str("<span></span>"),
    }
    body.push_str(&clone_menu(announcement));
    body.push_str("</div>");
    let mut csp = false;
    match (state, &default_branch) {
        (Some(state), Some(branch)) => {
            let segments: Vec<String> = branch.split('/').map(str::to_owned).collect();
            match git::resolve(state, &segments) {
                Some((ref_name, commit, _)) => {
                    body.push_str(&git::mount(
                        assets.as_ref(),
                        git::PageKind::Tree,
                        &base,
                        &ref_name,
                        &commit,
                        &[],
                        0,
                        &git::candidates(announcement, state, &defaults),
                    ));
                    csp = true;
                }
                None => body.push_str(
                    "<div class=\"box\"><div class=\"empty\">The default branch names no commit.</div></div>",
                ),
            }
        }
        _ => body
            .push_str("<div class=\"box\"><div class=\"empty\">No refs published yet.</div></div>"),
    }
    body.push_str("</div>");
    body.push_str(&about(&owner, announcement, state, &authors, &names));
    body.push_str("</div>");
    let mut response = html(StatusCode::OK, repo_name(announcement), &body);
    if csp {
        response.headers_mut().insert(
            header::CONTENT_SECURITY_POLICY,
            axum::http::HeaderValue::from_static(git::CONTENT_SECURITY_POLICY_VALUE),
        );
    }
    response
}

/// The green Code button: the repo's clone URLs, in a panel.
fn clone_menu(announcement: &Event) -> String {
    let urls = tag_values(announcement, "clone");
    let mut out = format!(
        "<details class=\"menu\"><summary class=\"btn btn-success\">{}Code{}</summary>\
         <div class=\"menu-panel right\"><div class=\"menu-title\">Clone</div><div class=\"menu-body\">",
        svg(icon::CODE, "icon-sm"),
        svg(icon::CHEVRON, "icon-sm caret"),
    );
    if urls.is_empty() {
        out.push_str("<p class=\"muted small\">This repository names no clone URL.</p>");
    }
    for url in urls {
        out.push_str(&format!(
            "<code class=\"copy-field\">git clone {}</code>",
            escape(url)
        ));
    }
    out.push_str(
        "<p class=\"muted small\">Pushing, issues and pull requests are paid writes, \
         sent with a TOON client.</p></div></div></details>",
    );
    out
}

/// The sidebar: what the repo is, its refs, its links and who keeps it.
fn about(
    owner: &PublicKey,
    announcement: &Event,
    state: Option<&Event>,
    authors: &[PublicKey],
    names: &HashMap<PublicKey, String>,
) -> String {
    let mut out = String::from("<aside class=\"sidebar\"><div><h2>About</h2>");
    match first_tag(announcement, "description")
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        Some(description) => out.push_str(&format!("<p>{}</p>", escape(description))),
        None => out.push_str("<p class=\"muted\"><em>No description provided.</em></p>"),
    }
    let web = tag_values(announcement, "web");
    if !web.is_empty() {
        out.push_str("<ul class=\"facts\">");
        for url in web {
            out.push_str(&format!("<li>{}{}</li>", svg(icon::LINK, ""), link(url)));
        }
        out.push_str("</ul>");
    }
    out.push_str("</div>");
    if let Some(state) = state {
        let held = refs(state);
        let count = |prefix: &str| {
            held.iter()
                .filter(|(name, _)| name.starts_with(prefix))
                .count()
        };
        let (branches, tags) = (count("refs/heads/"), count("refs/tags/"));
        out.push_str(&format!(
            "<ul class=\"facts\"><li>{}<span>{branches} branch{}</span></li>\
             <li>{}<span>{tags} tag{}</span></li></ul>",
            svg(icon::BRANCH, ""),
            if branches == 1 { "" } else { "es" },
            svg(icon::TAG, ""),
            if tags == 1 { "" } else { "s" },
        ));
    }
    out.push_str(&format!(
        "<div><h3>{}</h3><ul>",
        if authors.len() > 1 {
            "Maintainers"
        } else {
            "Owner"
        }
    ));
    for key in std::iter::once(owner).chain(authors.iter().filter(|key| *key != owner)) {
        out.push_str(&format!(
            "<li>{} {}</li>",
            avatar(&plain_name(names, key), true),
            name(names, key)
        ));
    }
    out.push_str("</ul></div></aside>");
    out
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
