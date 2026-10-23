//! Forks and remixes (proposal 7 of `docs/social-for-builders-ux.md`): the
//! Forks tab of a repo's page, `/social/r/<npub>/<d>/forks`.
//!
//! Two lineages meet on it, each written by the one who copied:
//!
//! - **Repos** (NIP-34): a fork's announcement names its parent in a `u` tag
//!   (`ui_nip34::lineage`). The tab draws the chain of parents up to the
//!   origin, each parent's other forks, and this repo's forks
//!   [`FORK_DEPTH`] levels down. A repo with the same earliest unique commit
//!   (`r … euc`) but no `u` tag is listed apart, as "same first commit".
//! - **Sites** (NIP-5A, `5A.md`, verified at nostr-protocol/nips@d47a2ec):
//!   a copied site manifest names the site it was copied from in an `a` tag
//!   and the origin of the lineage in an `A` tag. The remixes of this repo
//!   are the copies, and copies of copies, of its own sites (the manifests
//!   its makers signed whose `source` is the repo, [`crate::demo`]).
//!
//! A copy is "identical" when its file list hashes to the same NIP-5A
//! aggregate as its parent's: the hash is computed here from the `path`
//! tags, never taken from the `x` tag a copier wrote. A site with this
//! repo's aggregate, a different author and no `a`/`A` tag is an uncredited
//! copy ("same site"); it is found by its `x` tag and then checked.
//!
//! The tab counts people: the distinct keys that forked or copied, the
//! repo's own makers left out.

use std::collections::{HashMap, HashSet};

use nostr::event::{Event, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use relay_ui::ReadOnlyStore;
use relay_ui::chrome::{self, escape, icon, svg};
use sha2::{Digest, Sha256};
use ui_nip34::card::{Links, RepoCard, clone_urls, plural};
use ui_nip34::lineage::ForkNetwork;
use ui_nip34::repo::{RepoAddress, first_tag, repo_name, supersedes};

use crate::demo;
use crate::handlers::{self, Choice};
use crate::kinds::{SITE_NAMED, SITE_ROOT};
use crate::ship::{Site, Source};

/// How many levels of forks below this repo the tree draws.
pub(crate) const FORK_DEPTH: usize = 3;
/// How many parents above this repo the tree walks.
const ANCESTORS: usize = 4;
/// How many forks of one repo, or copies of one site, the tree draws.
const BREADTH: usize = 25;
/// The most site manifests one lineage reads.
const SITE_SCAN: usize = 1_000;

/// The NIP-5A aggregate hash of a site manifest: SHA-256 over its `path`
/// tags as sorted `<sha256> <path>\n` lines, in lower-case hex; none for a
/// manifest with no `path` tag. A pure function.
pub(crate) fn aggregate(site: &Event) -> Option<String> {
    let mut lines: Vec<String> = site
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter_map(|tag| match tag {
            [name, path, hash, ..] if name == "path" => Some(format!("{hash} {path}\n")),
            _ => None,
        })
        .collect();
    if lines.is_empty() {
        return None;
    }
    lines.sort();
    let digest = Sha256::digest(lines.concat().as_bytes());
    Some(digest.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// The site a manifest's tag called `name` (`a` for the parent, `A` for the
/// origin) names. A pure function.
fn lineage_tag(site: &Event, name: &str) -> Option<Site> {
    site.tags
        .iter()
        .map(|tag| tag.as_slice())
        .find_map(|tag| match tag {
            [tag_name, value, ..] if tag_name == name => Site::parse(value),
            _ => None,
        })
}

/// The coordinate of the site a manifest is.
fn coordinate_of(site: &Event) -> Option<String> {
    Site::of(site).map(|site| site.coordinate())
}

/// The newest of each site among `sites`, by its address. A pure function.
fn newest_sites(sites: Vec<Event>) -> Vec<Event> {
    let mut held: HashMap<String, Event> = HashMap::new();
    for site in sites.into_iter().filter(demo::is_manifest) {
        let Some(coordinate) = coordinate_of(&site) else {
            continue;
        };
        match held.get(&coordinate) {
            Some(current) if !supersedes(&site, current) => {}
            _ => {
                held.insert(coordinate, site);
            }
        }
    }
    let mut sites: Vec<Event> = held.into_values().collect();
    sites.sort_by_key(|site| (site.created_at, site.id));
    sites
}

/// What the Forks tab draws for one repo.
#[derive(Debug, Clone)]
pub(crate) struct Lineage {
    repo: RepoAddress,
    makers: Vec<PublicKey>,
    network: ForkNetwork,
    /// This repo's own sites, oldest first.
    sites: Vec<Event>,
    /// The sites copied, directly or not, from one of `sites`' lineages.
    copies: Vec<Event>,
    /// Sites with one of `sites`' aggregates, by others, crediting nothing.
    uncredited: Vec<Event>,
}

impl Lineage {
    /// The lineage of `card`'s repo, from `network` and the site manifests
    /// given: the repo's own `sites`, the `family` of copies their lineages
    /// hold, and the `lookalikes` that share an aggregate. A pure function.
    pub(crate) fn new(
        card: &RepoCard,
        network: ForkNetwork,
        sites: Vec<Event>,
        family: Vec<Event>,
        lookalikes: Vec<Event>,
    ) -> Self {
        let sites = newest_sites(sites);
        let own: HashSet<String> = sites.iter().filter_map(coordinate_of).collect();
        let copies: Vec<Event> = newest_sites(family)
            .into_iter()
            .filter(|site| coordinate_of(site).is_some_and(|c| !own.contains(&c)))
            .filter(|site| lineage_tag(site, "a").is_some())
            .collect();
        let aggregates: HashSet<String> = sites.iter().filter_map(aggregate).collect();
        let uncredited = newest_sites(lookalikes)
            .into_iter()
            .filter(|site| {
                !card.is_maker(&site.pubkey)
                    && lineage_tag(site, "a").is_none()
                    && lineage_tag(site, "A").is_none()
                    && aggregate(site).is_some_and(|hash| aggregates.contains(&hash))
            })
            .collect();
        Self {
            repo: card.address().clone(),
            makers: card.authorized().to_vec(),
            network,
            sites,
            copies,
            uncredited,
        }
    }

    /// The lineage of `card`'s repo, read from the store.
    pub(crate) async fn load(store: &ReadOnlyStore, card: &RepoCard) -> Self {
        let network = ForkNetwork::load(store, card.announcement()).await;
        let sites = demo::maker_sites(store, card).await;
        let mut origins: Vec<String> = Vec::new();
        let mut hashes: Vec<String> = Vec::new();
        for site in &sites {
            for origin in [
                lineage_tag(site, "A").map(|origin| origin.coordinate()),
                coordinate_of(site),
            ]
            .into_iter()
            .flatten()
            {
                if !origins.contains(&origin) {
                    origins.push(origin);
                }
            }
            for hash in [
                aggregate(site),
                first_tag(site, "x").map(str::to_ascii_lowercase),
            ]
            .into_iter()
            .flatten()
            {
                if !hashes.contains(&hash) {
                    hashes.push(hash);
                }
            }
        }
        let kinds = [Kind::from(SITE_NAMED), Kind::from(SITE_ROOT)];
        let mut family = Vec::new();
        let mut lookalikes = Vec::new();
        if !origins.is_empty() {
            for letter in [SingleLetterTag::UPPERCASE_A, SingleLetterTag::LOWERCASE_A] {
                family.extend(
                    store
                        .query(
                            Filter::new()
                                .kinds(kinds)
                                .custom_tags(letter, origins.clone())
                                .limit(SITE_SCAN),
                        )
                        .await,
                );
            }
        }
        if !hashes.is_empty() {
            lookalikes = store
                .query(
                    Filter::new()
                        .kinds(kinds)
                        .custom_tags(SingleLetterTag::LOWERCASE_X, hashes)
                        .limit(SITE_SCAN),
                )
                .await;
        }
        Self::new(card, network, sites, family, lookalikes)
    }

    /// The copies whose `a` tag names the site at `coordinate`, oldest first.
    fn copies_of(&self, coordinate: &str) -> Vec<&Event> {
        self.copies
            .iter()
            .filter(|site| {
                lineage_tag(site, "a").is_some_and(|parent| parent.coordinate() == coordinate)
            })
            .collect()
    }

    /// The site held for `coordinate`, this repo's or a copy.
    fn site(&self, coordinate: &str) -> Option<&Event> {
        self.sites
            .iter()
            .chain(&self.copies)
            .find(|site| coordinate_of(site).as_deref() == Some(coordinate))
    }

    /// The remixes of this repo's sites, [`FORK_DEPTH`] levels down, each
    /// once, with whether it is byte for byte its parent.
    fn remixes(&self) -> Vec<(&Event, bool)> {
        let mut found: Vec<(&Event, bool)> = Vec::new();
        let mut seen: HashSet<String> = self.sites.iter().filter_map(coordinate_of).collect();
        let mut level: Vec<&Event> = self.sites.iter().collect();
        for _ in 0..FORK_DEPTH {
            let mut next = Vec::new();
            for parent in level {
                let Some(coordinate) = coordinate_of(parent) else {
                    continue;
                };
                for copy in self.copies_of(&coordinate) {
                    if coordinate_of(copy).is_some_and(|c| seen.insert(c)) {
                        found.push((copy, aggregate(copy) == aggregate(parent)));
                        next.push(copy);
                    }
                }
            }
            level = next;
        }
        found
    }

    /// The forks of this repo, [`FORK_DEPTH`] levels down.
    fn forks(&self) -> Vec<RepoAddress> {
        self.network.descendants(&self.repo, FORK_DEPTH)
    }

    /// The repos with this repo's first commit and no `u` tag.
    fn same_first_commit(&self) -> Vec<RepoAddress> {
        self.network.same_first_commit(&self.repo)
    }

    /// The Forks tab's count: the distinct people who forked or copied this
    /// repo or its sites, its makers left out.
    pub(crate) fn people(&self) -> usize {
        let mut keys: HashSet<PublicKey> = HashSet::new();
        for repo in self.forks().iter().chain(&self.same_first_commit()) {
            keys.insert(*repo.owner());
        }
        for (site, _) in self.remixes() {
            keys.insert(site.pubkey);
        }
        for site in &self.uncredited {
            keys.insert(site.pubkey);
        }
        keys.retain(|key| !self.makers.contains(key));
        keys.len()
    }

    /// Every key the tab names, for its profiles.
    pub(crate) fn keys(&self) -> Vec<PublicKey> {
        let mut keys: Vec<PublicKey> = Vec::new();
        let mut add = |key: PublicKey| {
            if !keys.contains(&key) {
                keys.push(key);
            }
        };
        for parent in self.network.ancestors(&self.repo, ANCESTORS) {
            add(*parent.address().owner());
            for fork in self.network.forks(parent.address()) {
                add(*fork.owner());
            }
        }
        for repo in self.forks().iter().chain(&self.same_first_commit()) {
            add(*repo.owner());
        }
        for site in self
            .sites
            .iter()
            .chain(&self.copies)
            .chain(&self.uncredited)
        {
            add(site.pubkey);
        }
        for site in &self.sites {
            for tag in ["a", "A"] {
                if let Some(parent) = lineage_tag(site, tag) {
                    add(*parent.author());
                }
            }
        }
        keys
    }

    /// The newest of this repo's own sites: what a remix copies.
    fn newest_site(&self) -> Option<&Event> {
        self.sites.last()
    }
}

/// What drawing the tab needs beyond the lineage.
pub(crate) struct Draw<'a> {
    pub(crate) links: &'a Links,
    pub(crate) names: &'a HashMap<PublicKey, String>,
    pub(crate) gateway: Option<&'a str>,
}

/// `<ul class="lineage">` around `items`, or nothing.
fn list(items: &[String]) -> String {
    if items.is_empty() {
        String::new()
    } else {
        format!("<ul class=\"lineage\">{}</ul>", items.concat())
    }
}

/// One node of the tree: its chip, who, what, and a note.
fn node(
    chip: &str,
    here: bool,
    who: &PublicKey,
    title: &str,
    note: &str,
    draw: &Draw<'_>,
) -> String {
    format!(
        "<div class=\"node{}\"{}><span class=\"chip{}\">{}</span>{}{}{}</div>",
        if here { " here" } else { "" },
        if here { " aria-current=\"true\"" } else { "" },
        if here { " chip-here" } else { "" },
        escape(chip),
        chrome::person_avatar(draw.links.apps(), draw.names, who, true),
        title,
        if note.is_empty() {
            String::new()
        } else {
            format!("<span class=\"muted small\">{note}</span>")
        },
    )
}

/// A repo's node: `owner / name` linking to its page when this relay holds
/// it, its description, and how many forks it has below the drawn depth.
fn repo_node(
    lineage: &Lineage,
    address: &RepoAddress,
    chip: &str,
    git_url: Option<&str>,
    extra: &str,
    draw: &Draw<'_>,
) -> String {
    let here = *address == lineage.repo;
    let held = lineage.network.announcement(address);
    let label = format!(
        "{} / {}",
        chrome::shown_name(draw.names, address.owner()),
        held.map_or(address.d(), repo_name)
    );
    let title = match (held, draw.links.repo_link(address)) {
        (Some(_), Some(href)) => format!(
            "<a class=\"strong\" href=\"{}\" data-repo-node=\"{}\">{}</a>",
            escape(&href),
            escape(&address.coordinate()),
            escape(&label)
        ),
        _ => format!(
            "<span class=\"strong\" data-repo-node=\"{}\">{}</span>",
            escape(&address.coordinate()),
            escape(&label)
        ),
    };
    let mut note: Vec<String> = Vec::new();
    match held {
        Some(announcement) => {
            if let Some(description) = first_tag(announcement, "description")
                .map(str::trim)
                .filter(|text| !text.is_empty())
            {
                note.push(escape(description));
            }
        }
        None => {
            note.push("not on this relay".to_owned());
            if let Some(url) = git_url {
                note.push(format!("<code>{}</code>", escape(url)));
            }
        }
    }
    if !extra.is_empty() {
        note.push(extra.to_owned());
    }
    node(
        chip,
        here,
        address.owner(),
        &title,
        &note.join(" &middot; "),
        draw,
    )
}

/// A site's node: who copied it, its title, its source repo, and a link to
/// try it through the gateway.
fn site_node(lineage: &Lineage, site: &Event, chip: &str, note: &str, draw: &Draw<'_>) -> String {
    let title_text = first_tag(site, "title")
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            (site.kind.as_u16() == SITE_NAMED).then(|| ui_nip34::repo::d_tag(site).to_owned())
        })
        .unwrap_or_else(|| "root site".to_owned());
    let mut title = format!(
        "<span class=\"strong\" data-site=\"{}\">{} / {}</span>",
        escape(&coordinate_of(site).unwrap_or_default()),
        escape(&chrome::shown_name(draw.names, &site.pubkey)),
        escape(&title_text)
    );
    let mut notes: Vec<String> = Vec::new();
    if let Some(description) = first_tag(site, "description")
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        notes.push(escape(description));
    }
    let source = site
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find_map(|tag| match tag {
            [name, value, ..] if name == "source" => Source::parse(value),
            _ => None,
        });
    if let Some(Source::Repo(repo)) = source
        && repo != lineage.repo
        && let Some(announcement) = lineage.network.announcement(&repo)
    {
        let label = format!(
            "{} / {}",
            chrome::shown_name(draw.names, repo.owner()),
            repo_name(announcement)
        );
        notes.push(match draw.links.repo_link(&repo) {
            Some(href) => format!("code: <a href=\"{}\">{}</a>", escape(&href), escape(&label)),
            None => format!("code: {}", escape(&label)),
        });
    }
    if !note.is_empty() {
        notes.push(note.to_owned());
    }
    if let Some(url) = Site::of(site).and_then(|site| site.url(draw.gateway?)) {
        title.push_str(&format!(
            "<a class=\"small\" href=\"{}\" rel=\"noopener noreferrer\">Try it</a>",
            escape(&url)
        ));
    }
    node(
        chip,
        false,
        &site.pubkey,
        &title,
        &notes.join(" &middot; "),
        draw,
    )
}

/// The forks of `address`, `depth` levels down, as list items.
fn fork_items(
    lineage: &Lineage,
    address: &RepoAddress,
    depth: usize,
    draw: &Draw<'_>,
) -> Vec<String> {
    let forks = lineage.network.forks(address);
    let mut items: Vec<String> = forks
        .iter()
        .take(BREADTH)
        .map(|fork| {
            let below = lineage.network.forks(fork).len();
            if depth > 1 {
                let children = fork_items(lineage, fork, depth - 1, draw);
                format!(
                    "<li>{}{}</li>",
                    repo_node(lineage, fork, "fork", None, "", draw),
                    list(&children)
                )
            } else {
                let extra = if below > 0 {
                    format!("{below} more {} below", plural(below, "fork", "forks"))
                } else {
                    String::new()
                };
                format!(
                    "<li>{}</li>",
                    repo_node(lineage, fork, "fork", None, &extra, draw)
                )
            }
        })
        .collect();
    if forks.len() > BREADTH {
        items.push(format!(
            "<li class=\"muted small\">and {} more</li>",
            forks.len() - BREADTH
        ));
    }
    items
}

/// The copies of the site at `parent`, `depth` levels down.
fn site_items(lineage: &Lineage, parent: &Event, depth: usize, draw: &Draw<'_>) -> Vec<String> {
    let Some(coordinate) = coordinate_of(parent) else {
        return Vec::new();
    };
    let copies = lineage.copies_of(&coordinate);
    let mut items: Vec<String> = copies
        .iter()
        .take(BREADTH)
        .map(|copy| {
            let identical = aggregate(copy) == aggregate(parent);
            let (chip, note) = if identical {
                ("identical copy", "the same files, byte for byte")
            } else {
                ("remix", "")
            };
            let children = if depth > 1 {
                site_items(lineage, copy, depth - 1, draw)
            } else {
                Vec::new()
            };
            format!(
                "<li>{}{}</li>",
                site_node(lineage, copy, chip, note, draw),
                list(&children)
            )
        })
        .collect();
    if copies.len() > BREADTH {
        items.push(format!(
            "<li class=\"muted small\">and {} more</li>",
            copies.len() - BREADTH
        ));
    }
    items
}

/// The tree: the parents up to the origin, each with its other forks, and
/// this repo with its forks, its sites' remixes and uncredited copies.
fn tree(lineage: &Lineage, card: &RepoCard, demo_url: Option<&str>, draw: &Draw<'_>) -> String {
    let mut extra: Vec<String> = Vec::new();
    if let Some(url) = demo_url {
        extra.push(format!(
            "<a href=\"{}\" rel=\"noopener noreferrer\">site live</a>",
            escape(url)
        ));
    }
    if let Some(site) = lineage.newest_site()
        && let Some(parent) = lineage_tag(site, "a")
    {
        let origin = lineage_tag(site, "A").filter(|origin| *origin != parent);
        let named = |site: &Site| {
            let title = lineage
                .site(&site.coordinate())
                .and_then(|held| first_tag(held, "title").map(str::to_owned));
            escape(&format!(
                "{} / {}",
                chrome::shown_name(draw.names, site.author()),
                title.unwrap_or_else(|| {
                    Some(site.coordinate())
                        .and_then(|c| c.rsplit(':').next().map(str::to_owned))
                        .filter(|d| !d.is_empty())
                        .unwrap_or_else(|| "root site".to_owned())
                })
            ))
        };
        extra.push(match origin {
            Some(origin) => format!(
                "its site is copied from {}, originally {}",
                named(&parent),
                named(&origin)
            ),
            None => format!("its site is copied from {}", named(&parent)),
        });
    }
    let mut children = fork_items(lineage, &lineage.repo, FORK_DEPTH, draw);
    for site in &lineage.sites {
        children.extend(site_items(lineage, site, FORK_DEPTH, draw));
    }
    for site in &lineage.uncredited {
        children.push(format!(
            "<li>{}</li>",
            site_node(
                lineage,
                site,
                "same site",
                "an identical deploy, byte for byte, that credits nothing (no NIP-5A <code>a</code> or <code>A</code> tag)",
                draw
            )
        ));
    }
    let mut inner = format!(
        "<li>{}{}</li>",
        repo_node(
            lineage,
            card.address(),
            "this repo",
            None,
            &extra.join(" &middot; "),
            draw
        ),
        list(&children)
    );
    let ancestors = lineage.network.ancestors(&lineage.repo, ANCESTORS);
    let mut below = lineage.repo.clone();
    for (index, parent) in ancestors.iter().enumerate() {
        let top = index + 1 == ancestors.len();
        let held = lineage.network.announcement(parent.address()).is_some();
        let chip = match (
            top,
            held,
            lineage.network.parent(parent.address()).is_some(),
        ) {
            (true, true, false) => "origin",
            (true, false, _) => "upstream",
            _ => "fork",
        };
        let siblings: Vec<String> = lineage
            .network
            .forks(parent.address())
            .iter()
            .filter(|fork| **fork != below)
            .take(BREADTH)
            .map(|fork| {
                format!(
                    "<li>{}</li>",
                    repo_node(lineage, fork, "fork", None, "", draw)
                )
            })
            .collect();
        let git_url = parent.git_url();
        inner = format!(
            "<li>{}<ul class=\"lineage\">{inner}{}</ul></li>",
            repo_node(lineage, parent.address(), chip, git_url, "", draw),
            siblings.concat()
        );
        below = parent.address().clone();
    }
    format!("<ul class=\"lineage lineage-root\">{inner}</ul>")
}

/// The Forks tab's main column.
pub(crate) fn tab(
    lineage: &Lineage,
    card: &RepoCard,
    demo_url: Option<&str>,
    draw: &Draw<'_>,
) -> String {
    let mut out = String::from(
        "<p class=\"muted small\">Repos point at the repo they forked (NIP-34 <code>u</code> tag). \
         Deployed sites point at the site they copied and at its origin (NIP-5A <code>a</code> and \
         <code>A</code> tags). A repo with the same first commit (<code>r … euc</code>) is grouped \
         here even when it has no <code>u</code> tag. Each is the copier's own claim, on this relay \
         only.</p>",
    );
    let empty = lineage.network.ancestors(&lineage.repo, 1).is_empty()
        && lineage.network.forks(&lineage.repo).is_empty()
        && lineage.remixes().is_empty()
        && lineage.uncredited.is_empty();
    if empty {
        out.push_str(
            "<div class=\"box\"><div class=\"empty\">No forks or remixes of this repo on this relay yet.</div></div>",
        );
    } else {
        out.push_str(&format!(
            "<section class=\"box lineage-box\" aria-label=\"Forks and remixes\">{}</section>",
            tree(lineage, card, demo_url, draw)
        ));
    }
    let same = lineage.same_first_commit();
    if !same.is_empty() {
        let items: Vec<String> = same
            .iter()
            .take(BREADTH)
            .map(|repo| {
                format!(
                    "<li>{}</li>",
                    repo_node(lineage, repo, "same first commit", None, "", draw)
                )
            })
            .collect();
        out.push_str(&format!(
            "<h2 class=\"section-title\">Same first commit</h2>\
             <p class=\"muted small\">These repos start from this repo's earliest unique commit but do not say \
             they are forks: the same project announced again, or a fork with no <code>u</code> tag.</p>\
             <div class=\"box lineage-box\">{}</div>",
            list(&items)
        ));
    }
    out
}

/// The sidebar's "Lineage at a glance".
pub(crate) fn glance(lineage: &Lineage) -> String {
    let forks = lineage.forks().len();
    let remixes = lineage.remixes();
    let identical = remixes.iter().filter(|(_, same)| *same).count() + lineage.uncredited.len();
    let remixed = remixes.len() - remixes.iter().filter(|(_, same)| *same).count();
    let same = lineage.same_first_commit().len();
    format!(
        "<div class=\"side-card\" data-glance><h2>Lineage at a glance</h2>\
         <p class=\"small\">{forks} {} &middot; {remixed} {} &middot; {identical} identical {}</p>\
         <p class=\"muted small\">{same} more with the same first commit. {} {} forked or copied it.</p></div>",
        plural(forks, "fork", "forks"),
        plural(remixed, "remix", "remixes"),
        plural(identical, "copy", "copies"),
        lineage.people(),
        plural(lineage.people(), "person", "people"),
    )
}

/// The sidebar's Remix card: a Remix link per recommended builder known to
/// clone `nostr://` repos ([`handlers::CLONERS`]), how to fork with git,
/// and that both are public and permanent.
pub(crate) fn remix_card(
    lineage: &Lineage,
    card: &RepoCard,
    choices: &[Choice],
    profiles: &chrome::Profiles,
    links: &Links,
) -> String {
    let repo = card.address();
    let urls = clone_urls(repo, links.relay_host(), None);
    let clone = urls
        .first()
        .map(|(url, _)| url.as_str())
        .unwrap_or_default();
    let mut out = format!(
        "<div class=\"side-card\" data-remix><h2>Remix {}</h2>",
        escape(card.name())
    );
    let remixes: Vec<String> = choices
        .iter()
        .filter_map(|choice| {
            handlers::remix_url(choice.handler(), clone).map(|url| {
                format!(
                    "<a class=\"btn btn-success\" href=\"{}\" rel=\"noopener noreferrer\" data-remix-link>{}Remix in {}</a>",
                    escape(&url),
                    svg(icon::FORK, "icon-sm"),
                    escape(&handlers::label(choice.handler(), profiles))
                )
            })
        })
        .collect();
    if remixes.is_empty() {
        out.push_str(
            "<p class=\"muted small\">No app builder known to clone a <code>nostr://</code> repo is \
             recommended here, so there is no Remix button.</p>",
        );
    } else {
        let parent = lineage
            .newest_site()
            .and_then(coordinate_of)
            .unwrap_or_else(|| repo.coordinate());
        out.push_str(&format!(
            "<p class=\"small\">Opens a copy in an app builder. A remix of its site names \
             <code>{}</code> in an <code>a</code> tag and the lineage's origin in an <code>A</code> \
             tag (NIP-5A).</p>{}",
            escape(&parent),
            remixes.concat()
        ));
    }
    out.push_str(&format!(
        "<h3>Fork with git</h3><code class=\"copy-field\">git clone {}</code>\
         <p class=\"muted small\">Then announce your copy as a kind 30617 with \
         <code>[\"u\", \"{}|&lt;git url&gt;\"]</code>.</p>\
         <p class=\"muted small\" data-permanent>Forks and remixes are public and permanent on Nostr. \
         A deletion request (NIP-09) hides one here, but other relays may keep it.</p></div>",
        escape(clone),
        escape(&repo.coordinate()),
    ));
    out
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;
    use nostr::types::Timestamp;
    use relay_ui::chrome::npub;

    use super::*;

    fn signed(keys: &Keys, kind: u16, at: u64, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .custom_created_at(Timestamp::from(at))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn the_aggregate_is_nip_5a_s_hash_of_the_sorted_path_lines() {
        let keys = Keys::generate();
        let index = "186ea5fd14e88fd1ac49351759e7ab906fa94892002b60bf7f5a428f28ca1c99";
        let icon = "fedcba0987654321fedcba0987654321fedcba0987654321fedcba0987654321";
        let site = signed(
            &keys,
            35_128,
            1,
            &[
                &["d", "s"],
                &["path", "/favicon.ico", icon],
                &["path", "/index.html", index],
                &["x", "nope", "aggregate"],
            ],
        );
        // NIP-5A's example inputs, hashed as its `sort | sha256sum` line does.
        let expected =
            "c2ff582b672a4c689c5e1753528f03dd31b95ec1fdcc3d82d25e7d91e8769638".to_owned();
        assert_eq!(aggregate(&site), Some(expected));
        // Order and other tags do not change it.
        let reordered = signed(
            &keys,
            15_128,
            2,
            &[
                &["path", "/index.html", index],
                &["title", "t"],
                &["path", "/favicon.ico", icon],
            ],
        );
        assert_eq!(aggregate(&site), aggregate(&reordered));
        assert_eq!(aggregate(&signed(&keys, 35_128, 1, &[&["d", "s"]])), None);
    }

    struct World {
        maya: Keys,
        card: RepoCard,
        site: Event,
    }

    fn world() -> World {
        let maya = Keys::generate();
        let announcement = signed(&maya, 30_617, 1, &[&["d", "tidepool"]]);
        let card = RepoCard::from_announcement(&announcement).unwrap();
        let source = format!("nostr://{}/tidepool", npub(&maya.public_key()));
        let site = signed(
            &maya,
            35_128,
            2,
            &[
                &["d", "tidepool"],
                &["path", "/index.html", &"f".repeat(64)],
                &["source", &source],
            ],
        );
        World { maya, card, site }
    }

    #[test]
    fn copies_are_remixes_or_identical_and_uncredited_lookalikes_are_found() {
        let World { maya, card, site } = world();
        let (leo, dan, iris, anon) = (
            Keys::generate(),
            Keys::generate(),
            Keys::generate(),
            Keys::generate(),
        );
        let parent = coordinate_of(&site).unwrap();
        let surf = signed(
            &leo,
            35_128,
            3,
            &[
                &["d", "surfcast"],
                &["a", &parent],
                &["A", &parent],
                &["path", "/index.html", &"e".repeat(64)],
            ],
        );
        let kite = signed(
            &dan,
            35_128,
            4,
            &[
                &["d", "kitepool"],
                &["a", &coordinate_of(&surf).unwrap()],
                &["A", &parent],
                &["path", "/index.html", &"d".repeat(64)],
            ],
        );
        let same = signed(
            &iris,
            35_128,
            5,
            &[
                &["d", "tidepool"],
                &["a", &parent],
                &["A", &parent],
                &["path", "/index.html", &"f".repeat(64)],
            ],
        );
        let lookalike = signed(
            &anon,
            35_128,
            6,
            &[
                &["d", "tidepool"],
                &["path", "/index.html", &"f".repeat(64)],
            ],
        );
        let stranger = signed(
            &anon,
            35_128,
            7,
            &[&["d", "other"], &["path", "/index.html", &"c".repeat(64)]],
        );
        let lineage = Lineage::new(
            &card,
            ForkNetwork::default(),
            vec![site.clone()],
            vec![surf.clone(), kite.clone(), same.clone(), site.clone()],
            vec![lookalike.clone(), stranger, site],
        );
        let remixes = lineage.remixes();
        assert_eq!(remixes.len(), 3);
        assert!(remixes.contains(&(&surf, false)));
        assert!(remixes.contains(&(&kite, false)), "a copy of a copy");
        assert!(remixes.contains(&(&same, true)));
        assert_eq!(lineage.uncredited.len(), 1);
        assert_eq!(lineage.uncredited[0].id, lookalike.id);
        assert_eq!(lineage.people(), 4, "leo, dan, iris and anon");
        assert!(!lineage.keys().is_empty());
        let _ = maya;
    }

    #[test]
    fn the_tree_draws_the_origin_this_repo_and_its_forks_as_nested_lists() {
        let World { maya, card, .. } = world();
        let (noaa, ruth) = (Keys::generate(), Keys::generate());
        let origin = signed(&noaa, 30_617, 1, &[&["d", "harmonics"]]);
        let origin_a = RepoAddress::of(&origin).unwrap();
        let tidepool = signed(
            &maya,
            30_617,
            2,
            &[
                &["d", "tidepool"],
                &[
                    "u",
                    &format!("{}|https://git.test/h", origin_a.coordinate()),
                ],
            ],
        );
        let fork = signed(
            &ruth,
            30_617,
            3,
            &[&["d", "tidepool"], &["u", &card.address().coordinate()]],
        );
        let network = ForkNetwork::from_announcements(&[origin, tidepool.clone(), fork]);
        let card = RepoCard::from_announcement(&tidepool).unwrap();
        let lineage = Lineage::new(&card, network, vec![], vec![], vec![]);
        let links = Links::default();
        let names = HashMap::new();
        let draw = Draw {
            links: &links,
            names: &names,
            gateway: None,
        };
        let html = tab(&lineage, &card, None, &draw);
        let origin_at = html.find(">origin<").expect("the origin");
        let here_at = html.find("aria-current=\"true\"").expect("this repo");
        let fork_at = html.find(">fork<").expect("the fork");
        assert!(origin_at < here_at && here_at < fork_at);
        assert_eq!(
            html.matches("<ul class=\"lineage").count(),
            3,
            "three levels"
        );
        assert_eq!(lineage.people(), 1);
    }
}
