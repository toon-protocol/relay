//! A tenant's Deployment (kind 30437, `docs/nips/nip-deployment.md`): the
//! shape a page accepts, the state it shows, and the page. The shape and the
//! state are pure functions of events; only the page reads the store.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use nostr::event::{Event, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use relay_ui::ReadOnlyStore;
use relay_ui::chrome::{self, App, Apps, Profiles, escape};
use relay_ui::meta::summary;
use ui_nip34::repo::{ANNOUNCEMENT, RepoAddress, authorized_authors, owner_announcement};

use crate::directory::{self, LISTING_KIND, LIVENESS_KIND, expiration, json};
use crate::provider::{parse_pubkey, provider_link};
use crate::{PREFIX, Pages, directory_filter, fallback, html};

/// A Deployment.
pub(crate) const DEPLOYMENT_KIND: u16 = 30_437;
/// An Eviction Notice.
pub(crate) const EVICTION_KIND: u16 = 4_433;
/// A Takeover.
pub(crate) const TAKEOVER_KIND: u16 = 30_433;

/// The longest an Eviction Notice's reason or message is drawn.
const NOTE_CHARS: usize = 200;

/// A Deployment that has the NIP's shape and has not expired.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Deployment {
    /// The tenant who signed it.
    pub(crate) signer: PublicKey,
    /// Its `d`: the environment's name.
    pub(crate) environment: String,
    pub(crate) repo: RepoAddress,
    pub(crate) commit: String,
    /// The Listing's provider and its `d`.
    pub(crate) listing: (PublicKey, String),
    pub(crate) primary: PublicKey,
    pub(crate) standby: Vec<PublicKey>,
    /// The workload id, lowercase hex.
    pub(crate) workload: String,
    /// The image digest, lowercase hex.
    pub(crate) image: Option<String>,
    /// The `https` URL people open it at.
    pub(crate) visit: Option<String>,
    pub(crate) expiration: u64,
    pub(crate) created_at: u64,
}

/// What the public events say about a Deployment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Standing {
    /// Served, and its primary has an unexpired Liveness.
    Running { until: u64 },
    /// Served, and its primary has none.
    ProviderDown,
    /// An Eviction Notice from a provider it names.
    Evicted {
        by: PublicKey,
        reason: Option<String>,
        message: Option<String>,
    },
    /// A Takeover from a standby it names; `winner` signed it.
    TakenOver { winner: PublicKey },
}

fn hex64(text: &str) -> bool {
    text.len() == 64
        && text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub(crate) fn tag_slices(event: &Event) -> impl Iterator<Item = &[String]> {
    event.tags.iter().map(|tag| tag.as_slice())
}

impl Deployment {
    /// `event` as a Deployment: `None` unless it has exactly the NIP's shape
    /// (one repository `a`, one `c`, one Listing `a`, one `primary` `p`, any
    /// number of `standby` `p`, one `workload` `x`, at most one `image` `x`,
    /// at most one `https` `r`, an `expiration`) and `expiration` is after
    /// `now`. A pure function.
    pub(crate) fn parse(event: &Event, now: u64) -> Option<Self> {
        if event.kind != Kind::from(DEPLOYMENT_KIND) {
            return None;
        }
        let expires = expiration(event).filter(|at| *at > now)?;
        let (mut d, mut commit, mut repo, mut listing) = (None, None, None, None);
        let (mut primary, mut standby) = (None, Vec::new());
        let (mut workload, mut image, mut visit) = (None, None, None);
        for tag in tag_slices(event) {
            match tag {
                [name, value, ..] if name == "d" && d.is_none() => d = Some(value.clone()),
                [name, value, ..] if name == "c" => {
                    let full = matches!(value.len(), 40 | 64)
                        && value.bytes().all(|b| b.is_ascii_hexdigit());
                    if !full || commit.replace(value.to_ascii_lowercase()).is_some() {
                        return None;
                    }
                }
                [name, value, ..] if name == "a" => {
                    if value.starts_with(&format!("{ANNOUNCEMENT}:")) {
                        if repo.replace(RepoAddress::parse(value)?).is_some() {
                            return None;
                        }
                    } else if let Some(rest) = value.strip_prefix(&format!("{LISTING_KIND}:")) {
                        let (key, name) = rest.split_once(':')?;
                        let key = PublicKey::from_hex(key).ok()?;
                        if listing.replace((key, name.to_owned())).is_some() {
                            return None;
                        }
                    }
                }
                [name, value, marker, ..] if name == "p" => {
                    let key = PublicKey::from_hex(value).ok()?;
                    match marker.as_str() {
                        "primary" if primary.replace(key).is_none() => {}
                        "standby" if !standby.contains(&key) => standby.push(key),
                        _ => return None,
                    }
                }
                [name, ..] if name == "p" => return None,
                [name, value, marker, ..] if name == "x" => {
                    let slot = match marker.as_str() {
                        "workload" => &mut workload,
                        "image" => &mut image,
                        _ => return None,
                    };
                    if !hex64(value) || slot.replace(value.clone()).is_some() {
                        return None;
                    }
                }
                [name, ..] if name == "x" => return None,
                [name, value, ..] if name == "r" => {
                    let url = value.strip_prefix("https://").filter(|rest| {
                        !rest.is_empty() && rest.bytes().all(|b| b.is_ascii_graphic())
                    })?;
                    if visit.replace(format!("https://{url}")).is_some() {
                        return None;
                    }
                }
                _ => {}
            }
        }
        // A standby is another member of the Standby Set, never the primary.
        if primary.is_some_and(|key| standby.contains(&key)) {
            return None;
        }
        Some(Self {
            signer: event.pubkey,
            environment: d?,
            repo: repo?,
            commit: commit?,
            listing: listing?,
            primary: primary?,
            standby,
            workload: workload?,
            image,
            visit,
            expiration: expires,
            created_at: event.created_at.as_secs(),
        })
    }

    /// Whether `key` is in the Deployment's `p` tags.
    pub(crate) fn names(&self, key: &PublicKey) -> bool {
        self.primary == *key || self.standby.contains(key)
    }

    /// The keys the Deployment itself names: its signer, its providers and
    /// its Listing's owner.
    pub(crate) fn keys(&self) -> Vec<PublicKey> {
        let mut keys = vec![self.signer];
        keys.extend(self.providers());
        keys.push(self.listing.0);
        keys
    }

    /// Every provider the Deployment names, primary first.
    pub(crate) fn providers(&self) -> impl Iterator<Item = &PublicKey> {
        std::iter::once(&self.primary).chain(&self.standby)
    }
}

/// The note an Eviction Notice carries under `field`: its content's JSON,
/// else a tag of that name.
pub(crate) fn note(event: &Event, field: &str) -> Option<String> {
    let text = json(&event.content)
        .get(field)
        .and_then(|v| v.as_str().map(str::to_owned))
        .or_else(|| {
            tag_slices(event).find_map(|tag| match tag {
                [name, value, ..] if name == field => Some(value.clone()),
                _ => None,
            })
        })?;
    Some(summary(&text, NOTE_CHARS)).filter(|text| !text.is_empty())
}

/// The Takeovers in `takeovers` that count for `deployment`: for its workload
/// id, from a standby it names, naming its primary.
pub(crate) fn valid_takeovers<'a>(
    deployment: &'a Deployment,
    takeovers: &'a [Event],
) -> impl Iterator<Item = &'a Event> {
    takeovers
        .iter()
        .filter(|e| e.kind == Kind::from(TAKEOVER_KIND))
        .filter(|e| deployment.standby.contains(&e.pubkey))
        .filter(|e| {
            tag_slices(e).any(
                |tag| matches!(tag, [name, value, ..] if name == "d" && *value == deployment.workload),
            )
        })
        .filter(|e| {
            json(&e.content).get("primary").and_then(|v| v.as_str())
                == Some(deployment.primary.to_hex().as_str())
        })
}

/// The state of `deployment`. `primary_live` is whether its primary has an
/// unexpired Liveness; `evictions` and `takeovers` are any 4433 and 30433
/// events, of which only those the Deployment's own keys signed count. Of a
/// valid Eviction Notice and a valid Takeover, the newer stands (an Eviction
/// Notice, on a tie). Both beat liveness. A pure function.
pub(crate) fn state(
    deployment: &Deployment,
    primary_live: bool,
    evictions: &[Event],
    takeovers: &[Event],
) -> Standing {
    let evicted = evictions
        .iter()
        .filter(|e| e.kind == Kind::from(EVICTION_KIND))
        .filter(|e| deployment.names(&e.pubkey))
        .filter(|e| {
            tag_slices(e).any(
                |tag| matches!(tag, [name, value, ..] if name == "x" && *value == deployment.workload),
            )
        })
        .max_by_key(|e| (e.created_at, e.id));
    let taken = valid_takeovers(deployment, takeovers).max_by_key(|e| (e.created_at, e.id));
    match (evicted, taken) {
        (Some(e), Some(t)) if t.created_at > e.created_at => {
            Standing::TakenOver { winner: t.pubkey }
        }
        (Some(e), _) => Standing::Evicted {
            by: e.pubkey,
            reason: note(e, "reason"),
            message: note(e, "message"),
        },
        (None, Some(t)) => Standing::TakenOver { winner: t.pubkey },
        (None, None) if primary_live => Standing::Running {
            until: deployment.expiration,
        },
        (None, None) => Standing::ProviderDown,
    }
}

/// A Deployment's page: `/compute/d/<npub>/<environment>`.
pub(crate) fn deployment_path(signer: &PublicKey, environment: &str) -> String {
    format!(
        "{PREFIX}/d/{}/{}",
        chrome::npub(signer),
        chrome::percent_encode(environment)
    )
}

/// The Deployments in `events` that have the shape and have not expired,
/// newest first, each with its state read from the store.
async fn with_states(store: &ReadOnlyStore, events: &[Event]) -> Vec<(Deployment, Standing)> {
    let now = crate::now();
    let mut deployments: Vec<Deployment> = events
        .iter()
        .filter_map(|e| Deployment::parse(e, now))
        .collect();
    deployments.sort_by_key(|d| std::cmp::Reverse(d.created_at));
    if deployments.is_empty() {
        return Vec::new();
    }
    let workloads: Vec<String> = deployments.iter().map(|d| d.workload.clone()).collect();
    let primaries: Vec<PublicKey> = deployments.iter().map(|d| d.primary).collect();
    let (evictions, takeovers, liveness) = tokio::join!(
        store.query(
            Filter::new()
                .kind(Kind::from(EVICTION_KIND))
                .custom_tags(SingleLetterTag::LOWERCASE_X, workloads.clone())
                .limit(crate::READ_LIMIT)
        ),
        store.query(
            Filter::new()
                .kind(Kind::from(TAKEOVER_KIND))
                .identifiers(workloads)
                .limit(crate::READ_LIMIT)
        ),
        store.query(directory_filter(LIVENESS_KIND).authors(primaries)),
    );
    deployments
        .into_iter()
        .map(|d| {
            let s = state(
                &d,
                directory::is_live(&liveness, &d.primary, now),
                &evictions,
                &takeovers,
            );
            (d, s)
        })
        .collect()
}

/// The Deployments that name `provider` in a `p` tag, each with its state.
pub(crate) async fn serving(pages: &Pages, provider: &PublicKey) -> Vec<(Deployment, Standing)> {
    let events = pages
        .store
        .query(
            directory_filter(DEPLOYMENT_KIND)
                .custom_tag(SingleLetterTag::LOWERCASE_P, provider.to_hex()),
        )
        .await;
    with_states(&pages.store, &events)
        .await
        .into_iter()
        .filter(|(d, _)| d.names(provider))
        .collect()
}

/// A state as a short phrase, naming keys by `names`.
fn state_html(state: &Standing, names: &HashMap<PublicKey, String>) -> String {
    match state {
        Standing::Running { until } => format!(
            "<span data-state=\"running\">running on a live provider until {}</span>",
            escape(&chrome::date(i64::try_from(*until).unwrap_or(i64::MAX)))
        ),
        Standing::ProviderDown => {
            "<span data-state=\"provider-down\">provider down</span>".to_owned()
        }
        Standing::Evicted {
            by,
            reason,
            message,
        } => {
            let mut out = format!(
                "<span data-state=\"evicted\">evicted by {}</span>",
                provider_link(names, by)
            );
            out.push_str(&notes_html(reason.as_deref(), message.as_deref()));
            out
        }
        Standing::TakenOver { winner } => format!(
            "<span data-state=\"taken-over\">taken over by {}</span>",
            provider_link(names, winner)
        ),
    }
}

/// An Eviction Notice's `reason` and `message`, each with a leading space.
pub(crate) fn notes_html(reason: Option<&str>, message: Option<&str>) -> String {
    let mut out = String::new();
    if let Some(reason) = reason {
        out.push_str(&format!(" <span data-reason>{}</span>", escape(reason)));
    }
    if let Some(message) = message {
        out.push_str(&format!(" <span data-message>{}</span>", escape(message)));
    }
    out
}

/// The keys a Deployment page names, to look their names up once.
fn keys(deployment: &Deployment, state: &Standing) -> Vec<PublicKey> {
    let mut keys = deployment.keys();
    match state {
        Standing::Evicted { by: key, .. } | Standing::TakenOver { winner: key } => keys.push(*key),
        _ => {}
    }
    keys.sort();
    keys.dedup();
    keys
}

/// The names of every key the `served` Deployments name, looked up once.
async fn names_of(
    store: &ReadOnlyStore,
    served: &[(Deployment, Standing)],
) -> HashMap<PublicKey, String> {
    let mut all: Vec<PublicKey> = served.iter().flat_map(|(d, s)| keys(d, s)).collect();
    all.sort();
    all.dedup();
    Profiles::load(store, &all).await.into_names()
}

/// A Deployment's commit, shown as `label`, linked into `/repo` while that
/// app is on.
fn commit_html(deployment: &Deployment, repo_on: bool, label: &str) -> String {
    match repo_on {
        true => format!(
            "<a class=\"mono\" href=\"{}/commit/{}\">{}</a>",
            deployment.repo.repo_path(),
            chrome::percent_encode(&deployment.commit),
            escape(label)
        ),
        false => format!("<span class=\"mono\">{}</span>", escape(label)),
    }
}

/// The Serving section of a provider page. `served` is [`serving`]'s answer for the provider.
pub(crate) async fn serving_section(pages: &Pages, served: &[(Deployment, Standing)]) -> String {
    let mut body = String::from("<h2>Serving</h2>");
    if served.is_empty() {
        body.push_str("<div class=\"box\"><div class=\"empty\">No Deployments name this provider.</div></div>");
        return body;
    }
    let names = names_of(&pages.store, served).await;
    body.push_str(
        "<div class=\"box\"><div class=\"table-wrap\"><table class=\"data-table\">\
         <thead><tr><th>Deployment</th><th>Repository</th><th>State</th></tr></thead><tbody>",
    );
    for (d, s) in served {
        body.push_str(&format!(
            "<tr data-deployment=\"{}\"><td class=\"strong\"><a href=\"{}\">{}</a> {}</td>\
             <td>{}</td><td>{}</td></tr>",
            escape(&d.environment),
            deployment_path(&d.signer, &d.environment),
            escape(&d.environment),
            chrome::name(&names, &d.signer),
            repo_link(pages, &d.repo),
            state_html(s, &names),
        ));
    }
    body.push_str("</tbody></table></div></div>");
    body
}

/// The Environments tab's body for `repo` on `/repo`: the Deployments of
/// `repo` signed by one of `authorized` (its owner and the maintainers its
/// owner's latest announcement names, ADR 0002). Another key's Deployment of
/// the same repository has its own page but is not listed.
pub(crate) async fn environments(
    store: &ReadOnlyStore,
    apps: &Apps,
    repo: &RepoAddress,
    authorized: &[PublicKey],
) -> String {
    let events = store
        .query(
            directory_filter(DEPLOYMENT_KIND)
                .custom_tag(SingleLetterTag::LOWERCASE_A, repo.coordinate())
                .authors(authorized.iter().copied()),
        )
        .await;
    let served: Vec<(Deployment, Standing)> = with_states(store, &events)
        .await
        .into_iter()
        .filter(|(d, _)| d.repo == *repo)
        .collect();
    if served.is_empty() {
        return "<div class=\"box\"><div class=\"empty\">No environments: the repository's \
                owner and maintainers have published no Deployment of it to this relay.</div></div>"
            .to_owned();
    }
    let names = names_of(store, &served).await;
    let repo_on = apps.serves(App::Repo);
    let mut body = String::from(
        "<div class=\"box\"><div class=\"table-wrap\"><table class=\"data-table\">\
         <thead><tr><th>Environment</th><th>Provider</th><th>Commit</th><th>State</th>\
         <th>Visit</th></tr></thead><tbody>",
    );
    for (d, s) in &served {
        let commit = commit_html(d, repo_on, &d.commit[..7]);
        let visit = match &d.visit {
            Some(url) => format!(
                "<a data-visit href=\"{0}\" rel=\"nofollow noopener\">Visit</a>",
                escape(url)
            ),
            None => String::new(),
        };
        body.push_str(&format!(
            "<tr data-environment=\"{}\"><td class=\"strong\"><a href=\"{}\">{}</a> {}</td>\
             <td>{}</td><td>{commit}</td><td>{}</td><td>{visit}</td></tr>",
            escape(&d.environment),
            deployment_path(&d.signer, &d.environment),
            escape(&d.environment),
            chrome::name(&names, &d.signer),
            provider_link(&names, &d.primary),
            state_html(s, &names),
        ));
    }
    body.push_str("</tbody></table></div></div>");
    body
}

/// The repository, linked into `/repo` while that app is on.
fn repo_link(pages: &Pages, repo: &RepoAddress) -> String {
    let label = escape(repo.d());
    match pages.apps.serves(App::Repo) {
        true => format!("<a href=\"{}\">{label}</a>", repo.repo_path()),
        false => label,
    }
}

/// `/compute/d/<npub>/<environment>`: one Deployment.
pub(crate) async fn deployment_page(
    State(pages): State<Pages>,
    Path((npub, environment)): Path<(String, String)>,
) -> Response {
    let Some(signer) = parse_pubkey(&npub) else {
        return fallback(State(pages)).await;
    };
    let events = pages
        .store
        .query(
            directory_filter(DEPLOYMENT_KIND)
                .author(signer)
                .identifier(environment.clone()),
        )
        .await;
    let events: Vec<Event> = events
        .into_iter()
        .max_by_key(|e| (e.created_at, e.id))
        .into_iter()
        .collect();
    let Some((d, s)) = with_states(&pages.store, &events)
        .await
        .into_iter()
        .find(|(d, _)| d.environment == environment)
    else {
        return fallback(State(pages)).await;
    };
    let announcements = pages
        .store
        .query(
            Filter::new()
                .kind(Kind::from(ANNOUNCEMENT))
                .author(*d.repo.owner())
                .identifier(d.repo.d().to_owned()),
        )
        .await;
    // The owner is named by the repository address itself; a maintainer only
    // by the owner's announcement.
    let authorized = d.signer == *d.repo.owner()
        || owner_announcement(d.repo.owner(), d.repo.d(), &announcements)
            .is_some_and(|a| authorized_authors(a).contains(&d.signer));
    let mut all = keys(&d, &s);
    all.push(*d.repo.owner());
    all.sort();
    all.dedup();
    let names = Profiles::load(&pages.store, &all).await.into_names();

    let commit = commit_html(&d, pages.apps.serves(App::Repo), &d.commit);
    let mut rows = vec![
        ("State", state_html(&s, &names)),
        (
            "Repository",
            format!(
                "{} by {}",
                repo_link(&pages, &d.repo),
                chrome::name(&names, d.repo.owner())
            ),
        ),
        ("Commit", commit),
        ("Provider", provider_link(&names, &d.primary)),
        (
            "Listing",
            format!(
                "<a href=\"{PREFIX}/{}/{}\">{}</a>",
                chrome::npub(&d.listing.0),
                chrome::percent_encode(&d.listing.1),
                escape(&d.listing.1)
            ),
        ),
    ];
    if !d.standby.is_empty() {
        let standby: Vec<String> = d.standby.iter().map(|k| provider_link(&names, k)).collect();
        rows.push(("Standby", standby.join(", ")));
    }
    if let Some(image) = &d.image {
        rows.push(("Image", format!("<code>sha256:{}</code>", escape(image))));
    }
    if let Some(url) = &d.visit {
        rows.push((
            "Visit",
            format!(
                "<a data-visit href=\"{0}\" rel=\"nofollow noopener\">{0}</a>",
                escape(url)
            ),
        ));
    }
    rows.push(("Signed by", chrome::name(&names, &d.signer)));
    let claim = match authorized {
        true => String::new(),
        false => format!(
            "<p class=\"muted small\" data-claim=\"signer\">This is {}'s claim: that key is not \
             the repository's owner or one of its maintainers.</p>",
            chrome::name(&names, &d.signer)
        ),
    };
    let title = format!("{} · {}", d.repo.d(), d.environment);
    let body = format!(
        "<p class=\"muted small\"><a href=\"{PREFIX}\">Compute</a></p>\
         <h1 class=\"page-title\">{}</h1>{claim}{}",
        escape(&title),
        crate::provider::facts(&rows),
    );
    html(&pages, StatusCode::OK, &title, &body)
}

/// `/compute/d/<npub>/`: the Deployment whose `d` is empty.
pub(crate) async fn unnamed_deployment_page(
    pages: State<Pages>,
    Path(npub): Path<String>,
) -> Response {
    deployment_page(pages, Path((npub, String::new()))).await
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;

    use super::*;

    const NOW: u64 = 1_000;

    fn event(keys: &Keys, kind: u16, content: &str, tags: Vec<Vec<String>>) -> Event {
        EventBuilder::new(Kind::from(kind), content)
            .tags(tags.into_iter().map(|t| Tag::parse(t).expect("a tag")))
            .finalize(keys)
            .expect("signs")
    }

    fn tag(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|p| (*p).to_owned()).collect()
    }

    type Change = Box<dyn Fn(&mut Vec<Vec<String>>)>;

    struct Cast {
        tenant: Keys,
        primary: Keys,
        standby: Keys,
    }

    fn cast() -> Cast {
        Cast {
            tenant: Keys::generate(),
            primary: Keys::generate(),
            standby: Keys::generate(),
        }
    }

    fn workload() -> String {
        "ab".repeat(32)
    }

    fn tags(c: &Cast) -> Vec<Vec<String>> {
        vec![
            tag(&["d", "production"]),
            tag(&[
                "a",
                &format!("30617:{}:site", c.tenant.public_key().to_hex()),
            ]),
            tag(&["c", &"1".repeat(40)]),
            tag(&[
                "a",
                &format!("30432:{}:small", c.primary.public_key().to_hex()),
            ]),
            tag(&["p", &c.primary.public_key().to_hex(), "primary"]),
            tag(&["p", &c.standby.public_key().to_hex(), "standby"]),
            tag(&["x", &workload(), "workload"]),
            tag(&["x", &"cd".repeat(32), "image"]),
            tag(&["r", "https://app.example.com"]),
            tag(&["expiration", "5000"]),
            tag(&["L", "toon.network"]),
        ]
    }

    fn deployment(c: &Cast) -> Deployment {
        Deployment::parse(&event(&c.tenant, 30_437, "", tags(c)), NOW).expect("a Deployment")
    }

    #[test]
    fn a_deployment_with_the_nips_shape_parses() {
        let c = cast();
        let d = deployment(&c);
        assert_eq!(d.environment, "production");
        assert_eq!(d.primary, c.primary.public_key());
        assert_eq!(d.standby, vec![c.standby.public_key()]);
        assert_eq!(d.visit.as_deref(), Some("https://app.example.com"));
        assert_eq!(d.image, Some("cd".repeat(32)));
        assert_eq!(d.expiration, 5000);
    }

    #[test]
    fn a_deployment_that_breaks_the_shape_is_not_a_deployment() {
        let c = cast();
        let broken: Vec<(&str, Change)> = vec![
            (
                "unmarked x",
                Box::new(|t| t.push(tag(&["x", &"ef".repeat(32)]))),
            ),
            (
                "second workload",
                Box::new(|t| t.push(tag(&["x", &"ef".repeat(32), "workload"]))),
            ),
            (
                "second image",
                Box::new(|t| t.push(tag(&["x", &"ef".repeat(32), "image"]))),
            ),
            (
                "no workload",
                Box::new(|t| t.retain(|g| g.get(2).map(String::as_str) != Some("workload"))),
            ),
            ("no commit", Box::new(|t| t.retain(|g| g[0] != "c"))),
            (
                "two commits",
                Box::new(|t| t.push(tag(&["c", &"2".repeat(40)]))),
            ),
            ("short commit", Box::new(|t| t.push(tag(&["c", "abc"])))),
            (
                "no primary",
                Box::new(|t| t.retain(|g| g.get(2).map(String::as_str) != Some("primary"))),
            ),
            (
                "unmarked p",
                Box::new(|t| t.push(tag(&["p", &Keys::generate().public_key().to_hex()]))),
            ),
            (
                "no listing",
                Box::new(|t| t.retain(|g| !g[1].starts_with("30432:"))),
            ),
            (
                "no repository",
                Box::new(|t| t.retain(|g| !g[1].starts_with("30617:"))),
            ),
            (
                "http r",
                Box::new(|t| t.push(tag(&["r", "http://app.example.com"]))),
            ),
            (
                "script r",
                Box::new(|t| t.push(tag(&["r", "javascript:alert(1)"]))),
            ),
            (
                "no expiration",
                Box::new(|t| t.retain(|g| g[0] != "expiration")),
            ),
            ("no d", Box::new(|t| t.retain(|g| g[0] != "d"))),
            (
                "primary as a standby",
                Box::new(|t| {
                    let primary = t
                        .iter()
                        .find(|g| g.get(2).map(String::as_str) == Some("primary"))
                        .expect("a primary")[1]
                        .clone();
                    t.push(tag(&["p", &primary, "standby"]));
                }),
            ),
        ];
        for (what, change) in broken {
            let mut t = tags(&c);
            change(&mut t);
            let e = event(&c.tenant, 30_437, "", t);
            assert_eq!(Deployment::parse(&e, NOW), None, "{what}");
        }
    }

    #[test]
    fn a_deployment_past_its_expiration_is_not_shown() {
        let c = cast();
        let e = event(&c.tenant, 30_437, "", tags(&c));
        assert!(Deployment::parse(&e, 4_999).is_some());
        assert_eq!(Deployment::parse(&e, 5_000), None);
    }

    fn eviction(by: &Keys, workload: &str) -> Event {
        event(
            by,
            4_433,
            r#"{"reason":"abuse","message":"mining"}"#,
            vec![tag(&["x", workload])],
        )
    }

    fn takeover(by: &Keys, d: &str, primary: &Keys) -> Event {
        event(
            by,
            30_433,
            &format!(r#"{{"primary":"{}"}}"#, primary.public_key().to_hex()),
            vec![tag(&["d", d])],
        )
    }

    #[test]
    fn running_while_the_primary_is_live_and_down_when_it_is_not() {
        let c = cast();
        let d = deployment(&c);
        assert_eq!(state(&d, true, &[], &[]), Standing::Running { until: 5000 });
        assert_eq!(state(&d, false, &[], &[]), Standing::ProviderDown);
    }

    #[test]
    fn an_eviction_from_a_named_provider_beats_liveness_and_carries_its_words() {
        let c = cast();
        let d = deployment(&c);
        for by in [&c.primary, &c.standby] {
            let got = state(&d, true, &[eviction(by, &workload())], &[]);
            assert_eq!(
                got,
                Standing::Evicted {
                    by: by.public_key(),
                    reason: Some("abuse".into()),
                    message: Some("mining".into())
                }
            );
        }
    }

    #[test]
    fn a_takeover_from_a_named_standby_beats_liveness_and_names_the_winner() {
        let c = cast();
        let d = deployment(&c);
        let got = state(
            &d,
            true,
            &[],
            &[takeover(&c.standby, &workload(), &c.primary)],
        );
        assert_eq!(
            got,
            Standing::TakenOver {
                winner: c.standby.public_key()
            }
        );
    }

    #[test]
    fn events_from_keys_the_deployment_does_not_name_change_nothing() {
        let c = cast();
        let d = deployment(&c);
        let stranger = Keys::generate();
        let evictions = [
            eviction(&stranger, &workload()),
            eviction(&c.tenant, &workload()),
            eviction(&c.primary, &"00".repeat(32)),
        ];
        let takeovers = [
            takeover(&stranger, &workload(), &c.primary),
            // The primary is not a standby, and cannot take over from itself.
            takeover(&c.primary, &workload(), &c.primary),
            // A standby taking over from someone else's primary.
            takeover(&c.standby, &workload(), &stranger),
            takeover(&c.standby, &"00".repeat(32), &c.primary),
        ];
        assert_eq!(
            state(&d, true, &evictions, &takeovers),
            Standing::Running { until: 5000 }
        );
        assert_eq!(
            state(&d, false, &evictions, &takeovers),
            Standing::ProviderDown
        );
    }
}
