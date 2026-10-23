//! Topics (proposal 9 of `docs/social-for-builders-ux.md`): `/social/t`, every
//! topic on this relay with the number of repos under it, and
//! `/social/t/<topic>`, those repos.
//!
//! A repo is under a topic in two ways, and the page says which:
//!
//! - its owner put the topic in a `t` tag of the announcement (NIP-34);
//! - someone labelled it (NIP-32): a kind 1985 with `["L", "#t"]`, an
//!   `["l", <topic>, "#t"]` and the repo's address in an `a` tag. `#t` is
//!   NIP-32's own namespace for "attach this standard tag to the target", so
//!   a label in it means exactly what the owner's `t` tag means. Labels in any
//!   other namespace are not topics. The labeller is always shown: a label is
//!   someone's claim, paid for with one write, not the owner's.
//!
//! Topics are compared lower-cased, with a leading `#` dropped, as GitHub
//! does. The [`Catalog`] is also what [`crate::trending`] ranks over: every
//! repo announced here, with the keys whose acts on it are the maker's own.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use nostr::event::{Event, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use relay_ui::chrome::{
    self, Profiles, avatar, escape, icon, percent_encode, plain_name, svg,
};
use ui_nip34::card::{Links, plural};
use ui_nip34::repo::{
    ANNOUNCEMENT, RepoAddress, authorized_authors, first_tag, repo_name, supersedes, topics,
};

use crate::kinds::LABEL;
use crate::trending::{self, Window};
use crate::{PREFIX, Pages, html};

/// The NIP-32 namespace whose labels are topics.
pub(crate) const TOPIC_NAMESPACE: &str = "#t";
/// The most announcements the catalog reads: the newest this many repos.
const REPO_LIMIT: usize = 5_000;
/// The most topic labels the catalog reads.
const LABEL_LIMIT: usize = 5_000;
/// The longest topic kept.
const TOPIC_MAX: usize = 64;
/// The most topics a feed's sidebar names.
const SIDEBAR_TOPICS: usize = 12;

/// A topic as pages compare it: trimmed, a leading `#` dropped, lower case,
/// and at most [`TOPIC_MAX`] bytes; none when nothing is left. A pure
/// function.
pub(crate) fn normalize(topic: &str) -> Option<String> {
    let topic = topic.trim();
    let topic = topic
        .strip_prefix('#')
        .unwrap_or(topic)
        .trim()
        .to_lowercase();
    (!topic.is_empty() && topic.len() <= TOPIC_MAX && !topic.contains(['/', '?', '#']))
        .then_some(topic)
}

/// `/social/t/<topic>`.
pub(crate) fn topic_path(topic: &str) -> String {
    format!("{PREFIX}/t/{}", percent_encode(topic))
}

/// What a kind 1985 says about topics: the topics it labels in the `#t`
/// namespace and the repos it labels with them. A label that names no `L` of
/// `#t`, or whose `l` lacks the `#t` mark, labels nothing here. A pure
/// function.
pub(crate) fn topic_labels(event: &Event) -> (Vec<String>, Vec<RepoAddress>) {
    if event.kind != Kind::from(LABEL) {
        return (Vec::new(), Vec::new());
    }
    let tags = || event.tags.iter().map(|tag| tag.as_slice());
    let in_namespace = tags()
        .any(|tag| matches!(tag, [name, value, ..] if name == "L" && value == TOPIC_NAMESPACE));
    if !in_namespace {
        return (Vec::new(), Vec::new());
    }
    let mut found: Vec<String> = Vec::new();
    for tag in tags() {
        if let [name, value, mark, ..] = tag
            && name == "l"
            && mark == TOPIC_NAMESPACE
            && let Some(topic) = normalize(value)
            && !found.contains(&topic)
        {
            found.push(topic);
        }
    }
    (found, RepoAddress::tagged(event, "a"))
}

/// One repo under a topic: whether its owner tagged it, and who labelled it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Tagged {
    pub(crate) repo: RepoAddress,
    pub(crate) own: bool,
    pub(crate) labellers: Vec<PublicKey>,
}

/// Every repo announced on this relay (the newest [`REPO_LIMIT`]) and the
/// topic labels on them.
#[derive(Debug, Clone, Default)]
pub(crate) struct Catalog {
    announcements: HashMap<RepoAddress, Event>,
    /// `(topic, repo, labeller)`, once each.
    labels: Vec<(String, RepoAddress, PublicKey)>,
}

impl Catalog {
    /// The catalog of this relay's store: two queries.
    pub(crate) async fn load(store: &relay_ui::ReadOnlyStore) -> Self {
        let announcements = store
            .query(
                Filter::new()
                    .kind(Kind::from(ANNOUNCEMENT))
                    .limit(REPO_LIMIT),
            )
            .await;
        let labels = store
            .query(
                Filter::new()
                    .kind(Kind::from(LABEL))
                    .custom_tag(SingleLetterTag::UPPERCASE_L, TOPIC_NAMESPACE)
                    .limit(LABEL_LIMIT),
            )
            .await;
        Self::from_events(&announcements, &labels)
    }

    /// The catalog of these events: each repo's newest announcement by its
    /// owner, and the labels that name a repo held here. A pure function.
    pub(crate) fn from_events(announcements: &[Event], labels: &[Event]) -> Self {
        let mut held: HashMap<RepoAddress, Event> = HashMap::new();
        for event in announcements {
            let Some(address) = RepoAddress::of(event) else {
                continue;
            };
            match held.get(&address) {
                Some(current) if !supersedes(event, current) => {}
                _ => {
                    held.insert(address, event.clone());
                }
            }
        }
        let mut found: Vec<(String, RepoAddress, PublicKey)> = Vec::new();
        for event in labels {
            let (topics, repos) = topic_labels(event);
            for repo in repos.iter().filter(|repo| held.contains_key(repo)) {
                for topic in &topics {
                    let entry = (topic.clone(), repo.clone(), event.pubkey);
                    if !found.contains(&entry) {
                        found.push(entry);
                    }
                }
            }
        }
        Self {
            announcements: held,
            labels: found,
        }
    }

    /// The repo's announcement, if it is announced here.
    pub(crate) fn announcement(&self, repo: &RepoAddress) -> Option<&Event> {
        self.announcements.get(repo)
    }

    /// Each repo's owner and maintainers (ADR 0002): whose acts on it are
    /// the maker's, not the crowd's.
    pub(crate) fn makers(&self) -> HashMap<RepoAddress, Vec<PublicKey>> {
        self.announcements
            .iter()
            .map(|(repo, announcement)| (repo.clone(), authorized_authors(announcement)))
            .collect()
    }

    /// The topics the owner gave the repo, normalized, once each.
    pub(crate) fn own_topics(&self, repo: &RepoAddress) -> Vec<String> {
        let mut found: Vec<String> = Vec::new();
        for topic in self
            .announcement(repo)
            .map(topics)
            .unwrap_or_default()
            .into_iter()
            .filter_map(normalize)
        {
            if !found.contains(&topic) {
                found.push(topic);
            }
        }
        found
    }

    /// Whether `repo` is under `topic`, by its owner or by a label.
    pub(crate) fn has_topic(&self, repo: &RepoAddress, topic: &str) -> bool {
        self.own_topics(repo).iter().any(|own| own == topic)
            || self
                .labels
                .iter()
                .any(|(label, labelled, _)| label == topic && labelled == repo)
    }

    /// The repos under `topic`, each with how it got there, in no order.
    pub(crate) fn tagged(&self, topic: &str) -> Vec<Tagged> {
        let mut found: Vec<Tagged> = self
            .announcements
            .keys()
            .filter(|repo| self.own_topics(repo).iter().any(|own| own == topic))
            .map(|repo| Tagged {
                repo: repo.clone(),
                own: true,
                labellers: Vec::new(),
            })
            .collect();
        for (label, repo, labeller) in &self.labels {
            if label != topic {
                continue;
            }
            match found.iter_mut().find(|tagged| tagged.repo == *repo) {
                Some(tagged) => {
                    if !tagged.labellers.contains(labeller) {
                        tagged.labellers.push(*labeller);
                    }
                }
                None => found.push(Tagged {
                    repo: repo.clone(),
                    own: false,
                    labellers: vec![*labeller],
                }),
            }
        }
        found
    }

    /// Every topic with the number of distinct repos under it, most first
    /// (a tie by name).
    pub(crate) fn counts(&self) -> Vec<(String, usize)> {
        let mut repos: HashMap<String, HashSet<&RepoAddress>> = HashMap::new();
        for repo in self.announcements.keys() {
            for topic in self.own_topics(repo) {
                repos.entry(topic).or_default().insert(repo);
            }
        }
        for (topic, repo, _) in &self.labels {
            repos.entry(topic.clone()).or_default().insert(repo);
        }
        let mut counted: Vec<(String, usize)> = repos
            .into_iter()
            .map(|(topic, repos)| (topic, repos.len()))
            .collect();
        counted.sort_by(|(a, x), (b, y)| y.cmp(x).then_with(|| a.cmp(b)));
        counted
    }
}

/// A topic chip linking to its page, marked when it is the page's own.
pub(crate) fn chip(topic: &str, count: Option<usize>, href: &str, current: bool) -> String {
    format!(
        "<a class=\"chip chip-topic{}\" href=\"{}\"{}>{}{}</a>",
        if current { " on" } else { "" },
        escape(href),
        if current {
            " aria-current=\"page\""
        } else {
            ""
        },
        escape(topic),
        count
            .map(|count| format!("<span class=\"count\">{count}</span>"))
            .unwrap_or_default(),
    )
}

/// The feed sidebar's Topics box: the topics with the most repos.
pub(crate) fn sidebar_card(catalog: &Catalog) -> String {
    let counts = catalog.counts();
    let mut out = String::from("<div class=\"side-card\" data-topics><h2>Topics</h2>");
    if counts.is_empty() {
        out.push_str("<p class=\"muted small\">No repo here has a topic yet.</p>");
    } else {
        out.push_str("<div class=\"chips\">");
        for (topic, count) in counts.iter().take(SIDEBAR_TOPICS) {
            out.push_str(&chip(topic, Some(*count), &topic_path(topic), false));
        }
        out.push_str("</div>");
    }
    out.push_str(&format!(
        "<p class=\"muted small\">From the repos' own <code>t</code> tags and other people's \
         NIP-32 labels. <a href=\"{PREFIX}/t\">All topics</a></p></div>"
    ));
    out
}

/// What the sidebar of a topic page says about where topics come from.
fn about_card(topic: Option<&str>) -> String {
    format!(
        "<div class=\"side-card\"><h2>{}</h2><p class=\"muted small\">A repo is under a topic when \
         its owner put it in a <code>t</code> tag of the announcement (NIP-34), or when someone \
         labelled the repo with a kind 1985 in NIP-32's <code>#t</code> namespace: \
         <code>[\"L\",\"#t\"]</code>, <code>[\"l\",\"&lt;topic&gt;\",\"#t\"]</code> and the repo in \
         an <code>a</code> tag. A label is the labeller's claim, so the page names who added it. \
         Topics are compared in lower case.</p></div>",
        match topic {
            Some(topic) => format!("About {}", escape(topic)),
            None => "About topics".to_owned(),
        }
    )
}

/// `/social/t`: every topic, with the number of repos under it.
pub(crate) async fn index(State(pages): State<Pages>) -> Response {
    let catalog = Catalog::load(&pages.store).await;
    let counts = catalog.counts();
    let mut body = String::from(
        "<div class=\"page-hero\"><h1>Topics</h1><p class=\"muted\">What the repos on this relay \
         are about, by their owners' tags and other people's labels.</p></div>\
         <div class=\"split\"><div class=\"main\">",
    );
    if counts.is_empty() {
        body.push_str(
            "<div class=\"box\"><div class=\"empty\">No repo here has a topic yet.</div></div>",
        );
    } else {
        body.push_str("<div class=\"box box-body\"><div class=\"chips topic-index\">");
        for (topic, count) in &counts {
            body.push_str(&chip(topic, Some(*count), &topic_path(topic), false));
        }
        body.push_str("</div></div>");
        body.push_str(&format!(
            "<p class=\"muted small\">{} {}; each count is of distinct repos.</p>",
            counts.len(),
            plural(counts.len(), "topic", "topics"),
        ));
    }
    body.push_str("</div><aside class=\"sidebar\">");
    body.push_str(&about_card(None));
    body.push_str("</aside></div>");
    html(&pages, StatusCode::OK, "Topics", None, &body)
}

/// `/social/t/<topic>`: the repos under one topic.
pub(crate) async fn page(State(pages): State<Pages>, Path(raw): Path<String>) -> Response {
    let Some(topic) = normalize(&raw) else {
        return crate::not_found(&pages);
    };
    let store = &pages.store;
    let catalog = Catalog::load(store).await;
    let tagged = catalog.tagged(&topic);
    let repos: Vec<RepoAddress> = tagged.iter().map(|tagged| tagged.repo.clone()).collect();
    let counts = trending::stars_and_forks(store, &repos).await;
    let mut keys: Vec<PublicKey> = Vec::new();
    for key in tagged.iter().flat_map(|tagged| {
        catalog
            .announcement(&tagged.repo)
            .map(authorized_authors)
            .unwrap_or_default()
            .into_iter()
            .chain(tagged.labellers.iter().copied())
    }) {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let profiles = Profiles::load(store, &keys).await;
    let mut rows: Vec<(&Tagged, usize, usize)> = tagged
        .iter()
        .map(|tagged| {
            let (stars, forks) = counts.get(&tagged.repo).copied().unwrap_or_default();
            (tagged, stars, forks)
        })
        .collect();
    rows.sort_by(|(a, x, _), (b, y, _)| {
        y.cmp(x)
            .then_with(|| a.repo.coordinate().cmp(&b.repo.coordinate()))
    });

    let links = pages.links();
    let context = Rows {
        links: &links,
        catalog: &catalog,
        names: profiles.names(),
    };
    let mut body = format!(
        "<div class=\"page-hero\"><h1>{}<span class=\"chip chip-topic on\">{}</span></h1>\
         <p class=\"muted\">Repos tagged {} by their owners or labelled by others.</p></div>\
         <div class=\"split\"><div class=\"main\"><div class=\"toolbar\"><span class=\"muted\">{} {}</span>\
         <a class=\"btn\" href=\"{}\">{}Trending in {} this week</a></div>",
        svg(icon::TAG, ""),
        escape(&topic),
        escape(&topic),
        rows.len(),
        plural(rows.len(), "repo", "repos"),
        escape(&trending::path(Window::Week, None, Some(&topic), None)),
        svg(icon::STAR, "icon-sm"),
        escape(&topic),
    );
    if rows.is_empty() {
        body.push_str(&format!(
            "<div class=\"box\"><div class=\"empty\">No repo on this relay is tagged or labelled {}.</div></div>",
            escape(&topic)
        ));
    } else {
        body.push_str("<div class=\"box\"><ol class=\"trend-list\">");
        for (tagged, stars, forks) in rows {
            let Some(announcement) = catalog.announcement(&tagged.repo) else {
                continue;
            };
            body.push_str(&row(&context, announcement, tagged, (stars, forks), &topic));
        }
        body.push_str("</ol></div>");
    }
    body.push_str("</div><aside class=\"sidebar\">");
    body.push_str(&about_card(Some(&topic)));
    body.push_str(&format!(
        "<div class=\"side-card\"><h2>Ordered by stars</h2><p class=\"muted small\">Distinct people \
         who starred each repo (a kind 7 <code>+</code> with the repo's <code>a</code> and \
         <code>k=30617</code>), then by address. <a href=\"{PREFIX}/t\">All topics</a></p></div>"
    ));
    body.push_str("</aside></div>");
    html(
        &pages,
        StatusCode::OK,
        &format!("Topic: {topic}"),
        None,
        &body,
    )
}

/// What a list of repo rows is drawn with.
pub(crate) struct Rows<'a> {
    pub(crate) links: &'a Links,
    pub(crate) catalog: &'a Catalog,
    pub(crate) names: &'a HashMap<PublicKey, String>,
}

/// One repo under a topic.
fn row(
    context: &Rows<'_>,
    announcement: &Event,
    tagged: &Tagged,
    (stars, forks): (usize, usize),
    topic: &str,
) -> String {
    let repo = &tagged.repo;
    let names = context.names;
    let mut out = format!(
        "<li class=\"trend-row\" data-repo=\"{}\"><div class=\"trend-main\">{}",
        escape(&repo.coordinate()),
        title_link(context.links, repo, names, announcement),
    );
    if let Some(description) = first_tag(announcement, "description")
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        out.push_str(&format!("<p class=\"muted\">{}</p>", escape(description)));
    }
    out.push_str("<div class=\"meta\">");
    out.push_str(&topic_chips(context.catalog, repo, Some(topic)));
    out.push_str(&counts_html(stars, forks));
    out.push_str(&built_by(names, &authorized_authors(announcement)));
    out.push_str("</div>");
    if tagged.own || !tagged.labellers.is_empty() {
        out.push_str("<p class=\"muted small\" data-source>");
        if tagged.own {
            out.push_str("Tagged by its owner.");
        }
        if !tagged.labellers.is_empty() {
            if tagged.own {
                out.push(' ');
            }
            out.push_str("Labelled by ");
            let labellers: Vec<String> = tagged
                .labellers
                .iter()
                .map(|key| {
                    format!(
                        "<a class=\"person-link\" href=\"{}\" data-labeller>{}</a>",
                        chrome::profile_path(key),
                        chrome::name(names, key)
                    )
                })
                .collect();
            out.push_str(&labellers.join(", "));
            out.push_str(" (NIP-32).");
        }
        out.push_str("</p>");
    }
    out.push_str("</div></li>");
    out
}

/// `owner / name`, linking to the repo's social page (else its `/repo`
/// page).
pub(crate) fn title_link(
    links: &Links,
    repo: &RepoAddress,
    names: &HashMap<PublicKey, String>,
    announcement: &Event,
) -> String {
    let href = links.social_path(repo).unwrap_or_else(|| repo.repo_path());
    format!(
        "<a class=\"trend-title\" href=\"{}\">{} / <span class=\"strong\">{}</span></a>",
        escape(&href),
        escape(&plain_name(names, repo.owner())),
        escape(repo_name(announcement)),
    )
}

/// The repo's own topics as chips linking to their pages, `current` marked.
pub(crate) fn topic_chips(catalog: &Catalog, repo: &RepoAddress, current: Option<&str>) -> String {
    let own = catalog.own_topics(repo);
    if own.is_empty() {
        return String::new();
    }
    let mut out = String::from("<span class=\"chips\">");
    for topic in own.iter().take(5) {
        out.push_str(&chip(
            topic,
            None,
            &topic_path(topic),
            current == Some(topic.as_str()),
        ));
    }
    out.push_str("</span>");
    out
}

/// Stars and forks, for a row's meta line.
pub(crate) fn counts_html(stars: usize, forks: usize) -> String {
    format!(
        "<span data-stars=\"{stars}\">{}{stars} {}</span><span data-forks=\"{forks}\">{}{forks} {}</span>",
        svg(icon::STAR, "icon-sm"),
        plural(stars, "star", "stars"),
        svg(icon::BRANCH, "icon-sm"),
        plural(forks, "fork", "forks"),
    )
}

/// "Built by" and the makers' avatars.
pub(crate) fn built_by(names: &HashMap<PublicKey, String>, makers: &[PublicKey]) -> String {
    let mut out = String::from("<span class=\"built-by\">Built by");
    for key in makers.iter().take(4) {
        out.push_str(&format!(
            "<a class=\"avatar-link\" href=\"{}\" title=\"{}\">{}</a>",
            chrome::profile_path(key),
            escape(&plain_name(names, key)),
            avatar(&plain_name(names, key), true)
        ));
    }
    out.push_str("</span>");
    out
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;
    use nostr::types::Timestamp;

    use super::*;

    fn signed(keys: &Keys, kind: u16, at: u64, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .custom_created_at(Timestamp::from(at))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn a_topic_is_compared_in_lower_case_without_its_hash() {
        assert_eq!(normalize(" #Rust "), Some("rust".to_owned()));
        assert_eq!(
            normalize("built-with:claude"),
            Some("built-with:claude".to_owned())
        );
        for bad in ["", "  ", "#", "a/b", "x?y", &"x".repeat(65)] {
            assert_eq!(normalize(bad), None, "{bad}");
        }
        assert_eq!(
            topic_path("built-with:claude"),
            "/social/t/built-with%3Aclaude"
        );
    }

    #[test]
    fn only_a_label_in_the_t_namespace_is_a_topic() {
        let keys = Keys::generate();
        let repo = format!("30617:{}:r", keys.public_key().to_hex());
        let (topics, repos) = topic_labels(&signed(
            &keys,
            1985,
            1,
            &[
                &["L", "#t"],
                &["l", "Rust", "#t"],
                &["l", "wasm", "#t"],
                &["a", &repo],
            ],
        ));
        assert_eq!(topics, ["rust", "wasm"]);
        assert_eq!(repos.len(), 1);
        // Another namespace, a missing mark, or another kind label no topic.
        for (kind, tags) in [
            (1985, vec![vec!["L", "lang"], vec!["l", "rust", "lang"]]),
            (1985, vec![vec!["L", "#t"], vec!["l", "rust"]]),
            (1, vec![vec!["L", "#t"], vec!["l", "rust", "#t"]]),
        ] {
            let tags: Vec<&[&str]> = tags.iter().map(Vec::as_slice).collect();
            let mut tags = tags;
            let a = ["a", repo.as_str()];
            tags.push(&a);
            assert!(topic_labels(&signed(&keys, kind, 1, &tags)).0.is_empty());
        }
    }

    #[test]
    fn a_topic_holds_owned_and_labelled_repos_once_each() {
        let (maya, kofi, dan) = (Keys::generate(), Keys::generate(), Keys::generate());
        let tide = format!("30617:{}:tide", maya.public_key().to_hex());
        let sail = format!("30617:{}:sail", kofi.public_key().to_hex());
        let unknown = format!("30617:{}:gone", dan.public_key().to_hex());
        let announcements = [
            signed(&maya, 30_617, 1, &[&["d", "tide"], &["t", "old"]]),
            signed(
                &maya,
                30_617,
                2,
                &[&["d", "tide"], &["t", "Rust"], &["t", "rust"]],
            ),
            signed(&kofi, 30_617, 1, &[&["d", "sail"], &["t", "nostr"]]),
        ];
        let label = |at, topic: &str, target: &str| {
            signed(
                &dan,
                1985,
                at,
                &[&["L", "#t"], &["l", topic, "#t"], &["a", target]],
            )
        };
        let labels = [
            label(3, "rust", &sail),
            label(4, "rust", &sail),
            label(5, "rust", &tide),
            label(6, "rust", &unknown),
        ];
        let catalog = Catalog::from_events(&announcements, &labels);
        assert_eq!(
            catalog.own_topics(&RepoAddress::parse(&tide).unwrap()),
            ["rust"]
        );
        let mut tagged = catalog.tagged("rust");
        tagged.sort_by_key(|tagged| tagged.repo.d().to_owned());
        assert_eq!(tagged.len(), 2, "a repo not announced here is not listed");
        assert_eq!(tagged[0].repo.d(), "sail");
        assert!(!tagged[0].own);
        assert_eq!(tagged[0].labellers, [dan.public_key()]);
        assert!(tagged[1].own);
        assert_eq!(tagged[1].labellers, [dan.public_key()]);
        assert_eq!(
            catalog.counts(),
            [("rust".to_owned(), 2), ("nostr".to_owned(), 1)]
        );
        assert!(catalog.has_topic(&RepoAddress::parse(&sail).unwrap(), "rust"));
        assert!(!catalog.has_topic(&RepoAddress::parse(&tide).unwrap(), "old"));
    }
}
