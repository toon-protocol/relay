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

pub mod card;
mod git;
pub mod lineage;
mod markup;
pub mod repo;
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
use nostr::nips::nip19::{FromBech32, Nip19Coordinate};
use relay_ui::chrome::{Apps, escape, names, npub, percent_encode, person, person_avatar};
use relay_ui::{AppUi, Assets, ReadOnlyStore};

use markup::{Tab, ago, icon, ref_menu, repo_header, svg};
use repo::{
    ANNOUNCEMENT, STATE, authorized_authors, d_tag, first_tag, head_branch, honoured_state, newest,
    owner_announcement, refs, repo_name, supersedes, tag_values,
};

/// The mount prefix.
pub const PREFIX: &str = "/repo";

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
    apps: Apps,
}

/// The app UI over `store`, with the built `assets` if the relay has them,
/// the operator's `defaults` for where a browser looks for git objects, and
/// the `apps` the relay serves (for the switcher and the links to `/social`).
pub fn app(store: ReadOnlyStore, assets: Option<Assets>, defaults: Defaults, apps: Apps) -> AppUi {
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
        .fallback(fallback)
        .with_state(Pages {
            store,
            assets,
            defaults,
            apps,
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

fn repo_path(owner: &PublicKey, d: &str) -> String {
    format!("{PREFIX}/{}/{}", npub(owner), percent_encode(d))
}

fn html(apps: &Apps, status: StatusCode, title: &str, body: &str) -> Response {
    (status, Html(markup::page(apps, title, body))).into_response()
}

async fn fallback(State(pages): State<Pages>) -> Response {
    not_found(&pages.apps)
}

fn not_found(apps: &Apps) -> Response {
    html(
        apps,
        StatusCode::NOT_FOUND,
        "Not found",
        "<div class=\"box\"><div class=\"empty\"><h2>Not found</h2>\
         <p>Nothing on this relay lives at this address.</p>\
         <p><a href=\"/repo\">All repositories</a></p></div></div>",
    )
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

async fn list(
    State(Pages {
        store,
        assets,
        apps,
        ..
    }): State<Pages>,
) -> Response {
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
                person(&apps, &names, &repo.pubkey),
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
    html(&apps, StatusCode::OK, "Repositories", &body)
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

async fn home(
    State(Pages {
        store,
        assets,
        defaults,
        apps,
    }): State<Pages>,
    Path((owner, d)): Path<(String, String)>,
) -> Response {
    let Some((owner, announcement)) = resolve_repo(&store, &owner, &d).await else {
        return not_found(&apps);
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
    let names = names(&store, &lineage::named_keys(announcement)).await;
    let base = repo_path(&owner, &d);

    let mut body = repo_header(&apps, &owner, &d, announcement, &names, Tab::Code);
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
    body.push_str(&about(&apps, &owner, announcement, state, &authors, &names));
    body.push_str("</div>");
    let mut response = html(&apps, StatusCode::OK, repo_name(announcement), &body);
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
    apps: &Apps,
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
            person_avatar(apps, names, key, true),
            person(apps, names, key)
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
    State(Pages { store, apps, .. }): State<Pages>,
    Path(target): Path<String>,
) -> Response {
    if let Ok(address) = Nip19Coordinate::from_bech32(&target) {
        if address.kind == Kind::from(ANNOUNCEMENT) {
            return moved(&repo_path(&address.public_key, &address.identifier));
        }
        return not_found(&apps);
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
        return not_found(&apps);
    };
    let events = store.query(Filter::new().id(id)).await;
    let Some(event) = events.iter().find(|event| event.id == id) else {
        return not_found(&apps);
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
    not_found(&apps)
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
