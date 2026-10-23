//! App listings (NIP-82, still the open draft nostr-protocol/nips#1336, read
//! at its head `9d959cd`): `/social/apps`, every software application
//! (kind 32267) on this relay with a `?t=` topic filter; `/social/app/<naddr>`
//! and `/social/app/<npub>/<d>`, one app; `/social/apps/sets/<naddr>`, a
//! NIP-51 app curation set (kind 30267, Zapstore's "stacks").
//!
//! An app page shows what the app event says (icon, screenshots, name,
//! summary, Markdown description, license, website, source), its releases
//! (kind 30063 by the same author whose `i` is the app's `d`), grouped by
//! channel `c`, the newest by version ([`crate::version`], Appendix D), and
//! each release's assets (kind 3063, named by the release's `e` tags): their
//! platform, type, size, SHA-256 and the commit they say they were built
//! from. A download link appears only for an `https` URL; otherwise the file
//! is fetched by its hash from Blossom, which this relay does not do.
//!
//! Trust is social, as Zapstore has it: "who vouches" counts the distinct
//! people who starred the app (a kind 7 `+` on its address) or put it in an
//! app set, optionally only those a `?as=<npub>` follows. Reports from those
//! people raise a banner ([`crate::reports`]). The relay never fetches an
//! icon, a screenshot or an asset: the visitor's browser loads the `https`
//! images, under the `/social` policy.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use nostr::event::{Event, EventId, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use nostr::nips::nip01::Coordinate;
use nostr::nips::nip19::{FromBech32, Nip19Coordinate, ToBech32};
use nostr::types::Timestamp;
use relay_ui::ReadOnlyStore;
use relay_ui::chrome::{
    Profiles, ago, avatar, escape, icon, name, npub, percent_encode, plain_name, svg,
};
use ui_nip34::card::{Links, RepoCard, plural};
use ui_nip34::repo::{RepoAddress, first_tag, newest, supersedes};

use crate::kinds::{APP_SET, REACTION, SOFTWARE_APP, SOFTWARE_ASSET, SOFTWARE_RELEASE};
use crate::reports::{self, Targets};
use crate::viewer::{self, as_key, as_query, param};
use crate::{PREFIX, Pages, html, not_found, parse_pubkey, tips, version};

/// The most apps, releases or sets one query reads.
const LIMIT: usize = 500;
/// The most stars or set entries the vouching reads.
const PEOPLE_LIMIT: usize = 5_000;
/// The most vouchers whose avatars a page draws.
const FACES: usize = 12;
/// The draft's status, as every page that reads it says.
const DRAFT_NOTE: &str =
    "NIP-82 is still an open draft (nostr-protocol/nips#1336), not a merged NIP.";

/// An addressable event's address, `<kind>:<author hex>:<d>`, for the kinds
/// this module reads. The `d` may hold colons, so only the first two split.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct Address {
    kind: u16,
    author: PublicKey,
    d: String,
}

impl Address {
    /// The address, if `d` is not empty.
    pub(crate) fn new(kind: u16, author: PublicKey, d: &str) -> Option<Self> {
        (!d.is_empty()).then(|| Self {
            kind,
            author,
            d: d.to_owned(),
        })
    }

    /// The address an `a` tag value spells, if it names `kind`.
    pub(crate) fn parse(coordinate: &str, kind: u16) -> Option<Self> {
        let mut parts = coordinate.splitn(3, ':');
        let (named, author, d) = (parts.next()?, parts.next()?, parts.next()?);
        (named.parse::<u16>().ok()? == kind).then_some(())?;
        Self::new(kind, PublicKey::from_hex(author).ok()?, d)
    }

    /// The address an `naddr` spells, if it names `kind`.
    pub(crate) fn from_naddr(text: &str, kind: u16) -> Option<Self> {
        let text = text.strip_prefix("nostr:").unwrap_or(text);
        let coordinate = Nip19Coordinate::from_bech32(text).ok()?.coordinate;
        (coordinate.kind == Kind::from(kind)).then_some(())?;
        Self::new(kind, coordinate.public_key, &coordinate.identifier)
    }

    /// The address of an addressable event.
    pub(crate) fn of(event: &Event) -> Option<Self> {
        Self::new(
            event.kind.as_u16(),
            event.pubkey,
            ui_nip34::repo::d_tag(event),
        )
    }

    /// The addresses of `kind` that `event` names in `a` tags, once each.
    pub(crate) fn tagged(event: &Event, kind: u16) -> Vec<Self> {
        let mut found: Vec<Self> = Vec::new();
        for tag in event.tags.iter().map(|tag| tag.as_slice()) {
            if let [name, value, ..] = tag
                && name == "a"
                && let Some(address) = Self::parse(value, kind)
                && !found.contains(&address)
            {
                found.push(address);
            }
        }
        found
    }

    /// `<kind>:<author hex>:<d>`.
    pub(crate) fn coordinate(&self) -> String {
        format!("{}:{}:{}", self.kind, self.author.to_hex(), self.d)
    }

    /// The author's key.
    pub(crate) fn author(&self) -> &PublicKey {
        &self.author
    }

    /// The `d` tag.
    pub(crate) fn d(&self) -> &str {
        &self.d
    }

    /// The `naddr` of the address, with no relay hint.
    pub(crate) fn naddr(&self) -> String {
        let coordinate = Coordinate {
            kind: Kind::from(self.kind),
            public_key: self.author,
            identifier: self.d.clone(),
        };
        Nip19Coordinate::new(coordinate, Vec::<nostr::types::RelayUrl>::new())
            .to_bech32()
            .unwrap_or_default()
    }
}

/// `url` if it is `https` with a host and holds nothing a URL should not.
fn https(url: &str) -> Option<&str> {
    crate::media::https(url)
}

/// A link a visitor may follow: `https`, or plain `http`.
fn web_link(url: &str) -> Option<&str> {
    let url = url.trim();
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))?;
    (!rest.is_empty()
        && !url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '<' | '>')))
    .then_some(url)
}

/// The values of every tag called `name`, in order.
fn all_values<'a>(event: &'a Event, name: &str) -> Vec<&'a str> {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter_map(|tag| match tag {
            [tag_name, value, ..] if tag_name == name && !value.trim().is_empty() => {
                Some(value.as_str())
            }
            _ => None,
        })
        .collect()
}

/// A software application (kind 32267) as its newest event describes it.
#[derive(Debug, Clone)]
pub(crate) struct App {
    event: Event,
    address: Address,
}

impl App {
    /// The app `event` is, if it is a kind 32267 with a `d` tag.
    pub(crate) fn of(event: Event) -> Option<Self> {
        (event.kind == Kind::from(SOFTWARE_APP)).then_some(())?;
        let address = Address::of(&event)?;
        Some(Self { event, address })
    }

    /// The app's address.
    pub(crate) fn address(&self) -> &Address {
        &self.address
    }

    /// The app's page.
    pub(crate) fn path(&self) -> String {
        format!(
            "{PREFIX}/app/{}/{}",
            npub(self.address.author()),
            percent_encode(self.address.d())
        )
    }

    /// The `name` tag, else the `d`.
    pub(crate) fn name(&self) -> &str {
        first_tag(&self.event, "name")
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(self.address.d())
    }

    fn summary(&self) -> Option<&str> {
        first_tag(&self.event, "summary")
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }

    fn icon(&self) -> Option<&str> {
        first_tag(&self.event, "icon").and_then(https)
    }

    fn images(&self) -> Vec<&str> {
        let mut found: Vec<&str> = Vec::new();
        for url in all_values(&self.event, "image")
            .into_iter()
            .filter_map(https)
        {
            if !found.contains(&url) {
                found.push(url);
            }
        }
        found
    }

    fn topics(&self) -> Vec<&str> {
        all_values(&self.event, "t")
    }

    fn website(&self) -> Option<&str> {
        first_tag(&self.event, "url").and_then(web_link)
    }

    fn repository(&self) -> Option<&str> {
        first_tag(&self.event, "repository").and_then(web_link)
    }

    /// The NIP-34 repository its `a` tag points at.
    pub(crate) fn repo(&self) -> Option<RepoAddress> {
        RepoAddress::tagged(&self.event, "a").into_iter().next()
    }

    fn platforms(&self) -> Vec<&str> {
        all_values(&self.event, "f")
    }

    fn license(&self) -> Option<&str> {
        first_tag(&self.event, "license")
            .map(str::trim)
            .filter(|text| !text.is_empty())
    }
}

/// The newest event of each app address among `events`, newest first. A
/// pure function.
pub(crate) fn latest_apps(events: Vec<Event>) -> Vec<App> {
    let mut held: HashMap<Address, Event> = HashMap::new();
    for event in events {
        if event.kind != Kind::from(SOFTWARE_APP) {
            continue;
        }
        let Some(address) = Address::of(&event) else {
            continue;
        };
        match held.get(&address) {
            Some(current) if !supersedes(&event, current) => {}
            _ => {
                held.insert(address, event);
            }
        }
    }
    let mut apps: Vec<App> = held.into_values().filter_map(App::of).collect();
    apps.sort_by(|a, b| (b.event.created_at, a.event.id).cmp(&(a.event.created_at, b.event.id)));
    apps
}

/// One release (kind 30063).
#[derive(Debug, Clone)]
pub(crate) struct Release {
    event: Event,
}

impl Release {
    /// The `version` tag, else what follows the `@` in the `d` tag.
    pub(crate) fn version(&self) -> &str {
        first_tag(&self.event, "version")
            .map(str::trim)
            .filter(|version| !version.is_empty())
            .or_else(|| {
                ui_nip34::repo::d_tag(&self.event)
                    .rsplit_once('@')
                    .map(|(_, version)| version)
            })
            .unwrap_or("")
    }

    /// The channel `c`, `main` when it is missing (the draft's default).
    pub(crate) fn channel(&self) -> &str {
        first_tag(&self.event, "c")
            .map(str::trim)
            .filter(|channel| !channel.is_empty())
            .unwrap_or("main")
    }

    /// The release date: the event's `created_at`, as the draft requires.
    pub(crate) fn at(&self) -> Timestamp {
        self.event.created_at
    }

    /// The assets the release names, in its order.
    fn asset_ids(&self) -> Vec<EventId> {
        let mut ids: Vec<EventId> = Vec::new();
        for value in all_values(&self.event, "e") {
            if let Ok(id) = EventId::from_hex(value)
                && !ids.contains(&id)
            {
                ids.push(id);
            }
        }
        ids
    }
}

/// The releases of `app` among `events`: kind 30063 by the app's author
/// whose `i` is the app's `d`, the newest event of each `d`. A pure
/// function.
pub(crate) fn releases_of(app: &Address, events: Vec<Event>) -> Vec<Release> {
    let mut held: HashMap<String, Event> = HashMap::new();
    for event in events {
        if event.kind != Kind::from(SOFTWARE_RELEASE)
            || event.pubkey != *app.author()
            || first_tag(&event, "i") != Some(app.d())
        {
            continue;
        }
        let d = ui_nip34::repo::d_tag(&event).to_owned();
        match held.get(&d) {
            Some(current) if !supersedes(&event, current) => {}
            _ => {
                held.insert(d, event);
            }
        }
    }
    held.into_values().map(|event| Release { event }).collect()
}

/// Where a channel sorts: the draft's standard channels in its order, then
/// the rest by name.
fn channel_rank(channel: &str) -> (u8, String) {
    let rank = match channel {
        "main" => 0,
        "beta" => 1,
        "nightly" => 2,
        "dev" => 3,
        _ => 4,
    };
    (rank, channel.to_owned())
}

/// The channels among `releases`, in [`channel_rank`] order. A pure
/// function.
pub(crate) fn channels(releases: &[Release]) -> Vec<String> {
    let mut found: Vec<String> = Vec::new();
    for release in releases {
        if !found.iter().any(|held| held == release.channel()) {
            found.push(release.channel().to_owned());
        }
    }
    found.sort_by_key(|channel| channel_rank(channel));
    found
}

/// The releases on `channel`, newest version first (a tie by date, then
/// by id). A pure function.
pub(crate) fn on_channel<'a>(releases: &'a [Release], channel: &str) -> Vec<&'a Release> {
    let mut found: Vec<&Release> = releases
        .iter()
        .filter(|release| release.channel() == channel)
        .collect();
    found.sort_by(|a, b| {
        version::compare(b.version(), a.version())
            .then_with(|| b.event.created_at.cmp(&a.event.created_at))
            .then_with(|| a.event.id.cmp(&b.event.id))
    });
    found
}

/// The release a reader means by "latest": the newest on `main`, else on
/// the first channel there is.
pub(crate) fn latest_release(releases: &[Release]) -> Option<&Release> {
    let channel = channels(releases).into_iter().next()?;
    on_channel(releases, &channel).into_iter().next()
}

/// A size in bytes as people read it, in decimal units. A pure function.
pub(crate) fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["kB", "MB", "GB", "TB"];
    if bytes < 1_000 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = "B";
    for next in UNITS {
        if value < 1_000.0 {
            break;
        }
        value /= 1_000.0;
        unit = next;
    }
    format!("{value:.1} {unit}")
}

/// A hash shortened as `3f9a…c21e`, if it is hex. A pure function.
pub(crate) fn short_hash(hash: &str) -> Option<String> {
    (hash.len() >= 12 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        .then(|| format!("{}…{}", &hash[..4], &hash[hash.len() - 4..]))
}

/// The platform a MIME type implies (Appendix C of the draft).
fn mime_platform(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "application/vnd.android.package-archive" => "Android",
        "application/vnd.apple.ipa" => "iOS",
        "application/x-apple-diskimage"
        | "application/vnd.apple.installer+xml"
        | "application/x-mach-binary" => "macOS",
        "application/x-msi" | "application/vnd.microsoft.portable-executable" => "Windows",
        "application/vnd.appimage" | "application/x-executable" => "Linux",
        "application/vnd.flatpak" => "Linux (Flatpak)",
        "application/vnd.oci.image.manifest.v1+json" => "OCI container",
        "application/vsix" => "VS Code",
        "application/x-chrome-extension" => "Chrome",
        "application/x-xpinstall" => "Firefox",
        "application/wasm" => "Browser / WASI",
        "application/webbundle" => "Browser (PWA)",
        _ => return None,
    })
}

/// A platform identifier (Appendix A) as `OS architecture`. A pure function.
pub(crate) fn platform_label(id: &str) -> String {
    let (os, arch) = id.split_once('-').unwrap_or((id, ""));
    let os = match os {
        "android" => "Android",
        "darwin" => "macOS",
        "linux" => "Linux",
        "windows" => "Windows",
        "ios" => "iOS",
        "freebsd" => "FreeBSD",
        "wasm32" | "wasm64" => return format!("WebAssembly {}", &os[4..]),
        "wasi" => "WASI",
        other => other,
    };
    if arch.is_empty() {
        os.to_owned()
    } else {
        format!("{os} {arch}")
    }
}

/// A commit id a `commit` tag may hold: 40 or 64 hex.
fn commit_id(value: &str) -> Option<&str> {
    let value = value.trim();
    (matches!(value.len(), 40 | 64) && value.bytes().all(|b| b.is_ascii_hexdigit()))
        .then_some(value)
}

/// Whether `event` stars the app at `coordinate`: a kind 7 whose content
/// is `+`, naming the app in an `a` tag, with no `k` tag naming another
/// kind. A pure function.
pub(crate) fn is_app_star(event: &Event, coordinate: &str) -> bool {
    let tags = || event.tags.iter().map(|tag| tag.as_slice());
    event.kind == Kind::from(REACTION)
        && event.content.trim() == "+"
        && tags().any(|tag| matches!(tag, [name, value, ..] if name == "a" && value == coordinate))
        && !tags().any(|tag| {
            matches!(tag, [name, value, ..] if name == "k" && *value != SOFTWARE_APP.to_string())
        })
}

/// Whether an app set (kind 30267) lists the app at `coordinate`.
fn lists(set: &Event, coordinate: &str) -> bool {
    set.kind == Kind::from(APP_SET) && all_values(set, "a").contains(&coordinate)
}

/// The distinct people among `events` who vouch for the app at
/// `coordinate` (a star or a set listing it), newest first, only those in
/// `within` when it is given. A pure function.
pub(crate) fn vouchers(
    events: &[Event],
    coordinate: &str,
    within: Option<&HashSet<PublicKey>>,
) -> Vec<(PublicKey, Timestamp)> {
    ui_nip34::card::people(events, |event| {
        (is_app_star(event, coordinate) || lists(event, coordinate))
            && within.is_none_or(|keys| keys.contains(&event.pubkey))
    })
}

/// The newest app event at `address`.
async fn load_app(store: &ReadOnlyStore, address: &Address) -> Option<App> {
    let events = store
        .query(
            Filter::new()
                .kind(Kind::from(SOFTWARE_APP))
                .author(*address.author())
                .identifier(address.d()),
        )
        .await;
    newest(events.iter().filter(|event| {
        event.pubkey == *address.author() && ui_nip34::repo::d_tag(event) == address.d()
    }))
    .cloned()
    .and_then(App::of)
}

/// The releases of `app`.
async fn load_releases(store: &ReadOnlyStore, app: &Address) -> Vec<Release> {
    let events = store
        .query(
            Filter::new()
                .kind(Kind::from(SOFTWARE_RELEASE))
                .author(*app.author())
                .custom_tag(SingleLetterTag::LOWERCASE_I, app.d())
                .limit(LIMIT),
        )
        .await;
    releases_of(app, events)
}

/// The assets with `ids`, by id.
async fn load_assets(store: &ReadOnlyStore, ids: Vec<EventId>) -> HashMap<EventId, Event> {
    if ids.is_empty() {
        return HashMap::new();
    }
    store
        .query(Filter::new().kind(Kind::from(SOFTWARE_ASSET)).ids(ids))
        .await
        .into_iter()
        .filter(|event| event.kind == Kind::from(SOFTWARE_ASSET))
        .map(|event| (event.id, event))
        .collect()
}

/// The stars and set entries that may vouch for the apps at `coordinates`.
async fn load_vouches(store: &ReadOnlyStore, coordinates: Vec<String>) -> Vec<Event> {
    if coordinates.is_empty() {
        return Vec::new();
    }
    let mut events = store
        .query(
            Filter::new()
                .kind(Kind::from(REACTION))
                .custom_tags(SingleLetterTag::LOWERCASE_A, coordinates.clone())
                .limit(PEOPLE_LIMIT),
        )
        .await;
    let sets = store
        .query(
            Filter::new()
                .kind(Kind::from(APP_SET))
                .custom_tags(SingleLetterTag::LOWERCASE_A, coordinates)
                .limit(PEOPLE_LIMIT),
        )
        .await;
    events.extend(latest_sets(sets));
    events
}

/// The newest event of each app set among `events`. A pure function.
fn latest_sets(events: Vec<Event>) -> Vec<Event> {
    let mut held: HashMap<Address, Event> = HashMap::new();
    for event in events {
        if event.kind != Kind::from(APP_SET) {
            continue;
        }
        let Some(address) = Address::of(&event) else {
            continue;
        };
        match held.get(&address) {
            Some(current) if !supersedes(&event, current) => {}
            _ => {
                held.insert(address, event);
            }
        }
    }
    let mut sets: Vec<Event> = held.into_values().collect();
    sets.sort_by(|a, b| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
    sets
}

/// The apps the makers of `card`'s repo publish for it: kind 32267 by an
/// authorized author (ADR 0002) whose `a` tag names the repo.
pub(crate) async fn apps_of_repo(store: &ReadOnlyStore, card: &RepoCard) -> Vec<App> {
    let repo = card.address();
    let events = store
        .query(
            Filter::new()
                .kind(Kind::from(SOFTWARE_APP))
                .authors(card.authorized().to_vec())
                .custom_tag(SingleLetterTag::LOWERCASE_A, repo.coordinate())
                .limit(LIMIT),
        )
        .await;
    latest_apps(events)
        .into_iter()
        .filter(|app| card.is_maker(app.address().author()) && app.repo().as_ref() == Some(repo))
        .collect()
}

/// What a report about `app` may name: its address, each release (by
/// address and id) and each release's assets (by id and hash).
pub(crate) async fn add_targets(store: &ReadOnlyStore, app: &App, targets: &mut Targets) {
    let releases = load_releases(store, app.address()).await;
    let assets = load_assets(
        store,
        releases.iter().flat_map(Release::asset_ids).collect(),
    )
    .await;
    add_known_targets(app, &releases, &assets, targets);
}

/// [`add_targets`] for what is already loaded. A pure function.
fn add_known_targets(
    app: &App,
    releases: &[Release],
    assets: &HashMap<EventId, Event>,
    targets: &mut Targets,
) {
    targets.address(
        app.address().coordinate(),
        format!("the app {}", app.name()),
    );
    for release in releases {
        let label = format!("{} release {}", app.name(), release.version());
        if let Some(address) = Address::of(&release.event) {
            targets.address(address.coordinate(), label.clone());
        }
        targets.event(release.event.id, label);
        for id in release.asset_ids() {
            let Some(asset) = assets.get(&id) else {
                continue;
            };
            let label = format!("a {} {} asset", app.name(), release.version());
            targets.event(asset.id, label.clone());
            if let Some(hash) = first_tag(asset, "x") {
                targets.blob(hash, label);
            }
        }
    }
}

/// The repo page's "Latest release" card: the newest release of each app
/// the makers publish for the repo, or nothing when there is none.
pub(crate) async fn repo_sidebar(store: &ReadOnlyStore, card: &RepoCard) -> String {
    let apps = apps_of_repo(store, card).await;
    let mut rows = String::new();
    for app in apps.iter().take(3) {
        let releases = load_releases(store, app.address()).await;
        let Some(release) = latest_release(&releases) else {
            continue;
        };
        rows.push_str(&format!(
            "<li><a class=\"strong\" href=\"{}\">{}</a> <span class=\"mono\">{}</span>\
             <div class=\"muted small\">{} channel &middot; {}</div></li>",
            escape(&app.path()),
            escape(app.name()),
            escape(release.version()),
            escape(release.channel()),
            ago(release.at()),
        ));
    }
    if rows.is_empty() {
        if let Some(app) = apps.first() {
            return format!(
                "<div class=\"side-card\" data-latest-release><h2>App</h2><p><a href=\"{}\">{}</a></p>\
                 <p class=\"muted small\">Published for this repo; no release yet.</p></div>",
                escape(&app.path()),
                escape(app.name())
            );
        }
        return String::new();
    }
    format!(
        "<div class=\"side-card\" data-latest-release><h2>Latest release</h2><ul class=\"plain\">{rows}</ul>\
         <p class=\"muted small\">From the makers' app listing (NIP-82, an open draft).</p></div>"
    )
}

/// An app's icon: its `https` icon, else its initials.
fn icon_html(app: &App, large: bool) -> String {
    let class = if large {
        "app-icon app-icon-lg"
    } else {
        "app-icon"
    };
    match app.icon() {
        Some(url) => format!(
            "<img class=\"{class}\" src=\"{}\" alt=\"\" loading=\"lazy\" decoding=\"async\" referrerpolicy=\"no-referrer\">",
            escape(url)
        ),
        None => {
            let initials: String = app
                .name()
                .chars()
                .filter(|c| c.is_alphanumeric())
                .take(2)
                .collect::<String>()
                .to_uppercase();
            format!(
                "<span class=\"{class}\" aria-hidden=\"true\">{}</span>",
                escape(&initials)
            )
        }
    }
}

/// An app in a grid: icon, name, summary, topics, publisher, vouches.
fn tile(app: &App, names: &HashMap<PublicKey, String>, vouches: Option<usize>) -> String {
    let mut out = format!(
        "<article class=\"app-tile\" data-app=\"{}\">{}<div class=\"who\">\
         <a class=\"strong\" href=\"{}\">{}</a><span class=\"muted small\">by {}</span>",
        escape(&app.address().coordinate()),
        icon_html(app, false),
        escape(&app.path()),
        escape(app.name()),
        name(names, app.address().author()),
    );
    if let Some(summary) = app.summary() {
        out.push_str(&format!(
            "<p class=\"muted small\">{}</p>",
            escape(&crate::content::clip(summary, 160))
        ));
    }
    let topics = app.topics();
    if !topics.is_empty() {
        out.push_str("<div class=\"chips\">");
        for topic in topics.iter().take(4) {
            out.push_str(&format!(
                "<a class=\"chip chip-topic\" href=\"{PREFIX}/apps?t={}\">{}</a>",
                percent_encode(&topic.to_lowercase()),
                escape(topic)
            ));
        }
        out.push_str("</div>");
    }
    if let Some(count) = vouches {
        out.push_str(&format!(
            "<span class=\"muted small\" data-vouches=\"{count}\">{}{count} {} vouch</span>",
            svg(icon::STAR, "icon-sm"),
            plural(count, "person", "people")
        ));
    }
    out.push_str("</div></article>");
    out
}

/// `/social/apps`: every app here, newest first, `?t=` for one topic.
pub(crate) async fn index(State(pages): State<Pages>, RawQuery(query): RawQuery) -> Response {
    let store = &pages.store;
    let topic = param(query.as_deref(), "t")
        .map(|topic| topic.trim().trim_start_matches('#').to_lowercase())
        .filter(|topic| !topic.is_empty());
    let mut filter = Filter::new().kind(Kind::from(SOFTWARE_APP)).limit(LIMIT);
    if let Some(topic) = &topic {
        filter = filter.custom_tag(SingleLetterTag::LOWERCASE_T, topic.clone());
    }
    let apps: Vec<App> = latest_apps(store.query(filter).await)
        .into_iter()
        .filter(|app| {
            topic.as_ref().is_none_or(|topic| {
                app.topics()
                    .iter()
                    .any(|held| held.to_lowercase() == *topic)
            })
        })
        .collect();
    let vouches = load_vouches(
        store,
        apps.iter().map(|app| app.address().coordinate()).collect(),
    )
    .await;
    let sets = latest_sets(
        store
            .query(Filter::new().kind(Kind::from(APP_SET)).limit(50))
            .await,
    );
    let mut keys: Vec<PublicKey> = Vec::new();
    for key in apps
        .iter()
        .map(|app| *app.address().author())
        .chain(sets.iter().map(|set| set.pubkey))
    {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let profiles = Profiles::load(store, &keys).await;
    let names = profiles.names();

    let mut body = String::from("<div class=\"split\"><div class=\"main\">");
    body.push_str(&format!(
        "<h1 class=\"page-title\">Apps</h1><p class=\"muted small\">Software published to this relay as \
         NIP-82 app listings (kind 32267), each with its releases and files. {DRAFT_NOTE}</p>"
    ));
    let mut counts: HashMap<String, usize> = HashMap::new();
    for app in &apps {
        for held in app.topics() {
            *counts.entry(held.to_lowercase()).or_default() += 1;
        }
    }
    let mut ranked: Vec<(String, usize)> = counts.into_iter().collect();
    ranked.sort_by(|(a, x), (b, y)| y.cmp(x).then_with(|| a.cmp(b)));
    if topic.is_some() || !ranked.is_empty() {
        body.push_str("<div class=\"chips\" data-topics>");
        body.push_str(&format!(
            "<a class=\"chip{}\" href=\"{PREFIX}/apps\">All</a>",
            if topic.is_none() { " chip-current" } else { "" }
        ));
        if let Some(topic) = &topic
            && !ranked.iter().any(|(held, _)| held == topic)
        {
            ranked.insert(0, (topic.clone(), 0));
        }
        for (held, count) in ranked.iter().take(12) {
            body.push_str(&format!(
                "<a class=\"chip chip-topic{}\" href=\"{PREFIX}/apps?t={}\">{} <span class=\"count\">{count}</span></a>",
                if topic.as_ref() == Some(held) { " chip-current" } else { "" },
                percent_encode(held),
                escape(held)
            ));
        }
        body.push_str("</div>");
    }
    if apps.is_empty() {
        body.push_str("<div class=\"box\"><div class=\"empty\">No apps here yet.</div></div>");
    } else {
        body.push_str("<div class=\"app-grid\">");
        for app in &apps {
            let coordinate = app.address().coordinate();
            body.push_str(&tile(
                app,
                names,
                Some(vouchers(&vouches, &coordinate, None).len()),
            ));
        }
        body.push_str("</div>");
    }
    body.push_str("</div><aside class=\"sidebar\"><div class=\"side-card\"><h2>App sets</h2>");
    if sets.is_empty() {
        body.push_str("<p class=\"muted small\">No app sets yet.</p>");
    } else {
        body.push_str("<ul class=\"plain\">");
        for set in sets.iter().take(12) {
            let Some(address) = Address::of(set) else {
                continue;
            };
            body.push_str(&format!(
                "<li><a href=\"{PREFIX}/apps/sets/{}\">{}</a><div class=\"muted small\">{} {} &middot; by {}</div></li>",
                address.naddr(),
                escape(&set_title(set)),
                Address::tagged(set, SOFTWARE_APP).len(),
                plural(Address::tagged(set, SOFTWARE_APP).len(), "app", "apps"),
                name(names, &set.pubkey),
            ));
        }
        body.push_str("</ul>");
    }
    body.push_str(
        "<p class=\"muted small\">Curated lists of apps (NIP-51 kind 30267), what Zapstore calls \
         stacks.</p></div><div class=\"side-card\"><h2>Publishing an app</h2><p class=\"muted small\">\
         This page writes nothing. A client publishes a kind 32267 for the app, a kind 30063 per \
         release and a kind 3063 per file, each one paid write; <code>docs/apps/social.md</code> \
         has the shapes.</p></div></aside></div>",
    );
    let title = match &topic {
        Some(topic) => format!("Apps · {topic}"),
        None => "Apps".to_owned(),
    };
    html(
        &pages,
        crate::Nav::Apps,
        StatusCode::OK,
        &title,
        None,
        &body,
    )
}

/// A set's `title`, else its `d`.
fn set_title(set: &Event) -> String {
    first_tag(set, "title")
        .or_else(|| first_tag(set, "name"))
        .map(str::trim)
        .filter(|title| !title.is_empty())
        .unwrap_or_else(|| ui_nip34::repo::d_tag(set))
        .to_owned()
}

/// `/social/app/<naddr>`.
pub(crate) async fn by_naddr(
    State(pages): State<Pages>,
    Path(target): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    match Address::from_naddr(&target, SOFTWARE_APP) {
        Some(address) => app_page(&pages, &address, query.as_deref()).await,
        None => not_found(&pages),
    }
}

/// `/social/app/<npub>/<d>`.
pub(crate) async fn page(
    State(pages): State<Pages>,
    Path((owner, d)): Path<(String, String)>,
    RawQuery(query): RawQuery,
) -> Response {
    match parse_pubkey(&owner).and_then(|owner| Address::new(SOFTWARE_APP, owner, &d)) {
        Some(address) => app_page(&pages, &address, query.as_deref()).await,
        None => not_found(&pages),
    }
}

/// The address `/social/app/<npub>/<d>` names, for the reports page.
pub(crate) fn address_of_path(owner: &str, d: &str) -> Option<Address> {
    parse_pubkey(owner).and_then(|owner| Address::new(SOFTWARE_APP, owner, d))
}

/// The app at `address`, for the reports page.
pub(crate) async fn find(store: &ReadOnlyStore, address: &Address) -> Option<App> {
    load_app(store, address).await
}

async fn app_page(pages: &Pages, address: &Address, query: Option<&str>) -> Response {
    let store = &pages.store;
    let Some(app) = load_app(store, address).await else {
        return not_found(pages);
    };
    let viewer = as_key(query);
    let releases = load_releases(store, address).await;
    let assets = load_assets(
        store,
        releases.iter().flat_map(Release::asset_ids).collect(),
    )
    .await;
    let all_channels = channels(&releases);
    let channel = param(query, "channel")
        .filter(|wanted| all_channels.contains(wanted))
        .or_else(|| all_channels.first().cloned());
    let shown = channel
        .as_deref()
        .map(|channel| on_channel(&releases, channel))
        .unwrap_or_default();
    let card = match app.repo() {
        Some(repo) => RepoCard::load(store, &repo).await,
        None => None,
    };
    let coordinate = address.coordinate();
    let vouches = load_vouches(store, vec![coordinate.clone()]).await;
    let follows: Option<HashSet<PublicKey>> = match &viewer {
        Some(viewer) => Some(viewer::follows(store, viewer).await.into_iter().collect()),
        None => None,
    };
    let vouched = vouchers(&vouches, &coordinate, follows.as_ref());
    let sets: Vec<&Event> = vouches
        .iter()
        .filter(|event| lists(event, &coordinate))
        .collect();
    let mut targets = Targets::default();
    add_known_targets(&app, &releases, &assets, &mut targets);
    let found_reports = reports::load(store, &targets).await;

    let mut keys: Vec<PublicKey> = vec![*address.author()];
    for key in vouched
        .iter()
        .take(FACES)
        .map(|(key, _)| *key)
        .chain(sets.iter().map(|set| set.pubkey))
        .chain(tips::split_keys(&app.event))
        .chain(viewer)
    {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let profiles = Profiles::load(store, &keys).await;
    let names = profiles.names();
    let links = pages.links();
    let lens = as_query(viewer.as_ref(), false);

    let mut body = reports::banner(
        &found_reports,
        &targets,
        viewer.as_ref().zip(follows.as_ref()),
        names,
        Some(&format!(
            "{}/reports{}",
            app.path(),
            as_query(viewer.as_ref(), true)
        )),
    );
    body.push_str("<div class=\"split\"><div class=\"main\">");
    body.push_str(&format!(
        "<div class=\"app-head\" data-app=\"{}\">{}<div class=\"who\"><h1 class=\"app-title\">{}</h1>",
        escape(&coordinate),
        icon_html(&app, true),
        escape(app.name())
    ));
    if let Some(summary) = app.summary() {
        body.push_str(&format!("<p class=\"repo-desc\">{}</p>", escape(summary)));
    }
    body.push_str("<div class=\"chips\">");
    for topic in app.topics() {
        body.push_str(&format!(
            "<a class=\"chip chip-topic\" href=\"{PREFIX}/apps?t={}\">{}</a>",
            percent_encode(&topic.to_lowercase()),
            escape(topic)
        ));
    }
    if let Some(license) = app.license() {
        body.push_str(&format!("<span class=\"chip\">{}</span>", escape(license)));
    }
    body.push_str(&format!(
        "<span class=\"chip mono\">{}</span></div></div></div>",
        escape(address.d())
    ));

    let mut buttons = String::new();
    if let Some(url) = app.website() {
        buttons.push_str(&format!(
            "<a class=\"btn\" href=\"{}\" rel=\"nofollow noopener noreferrer\" data-website>{}Website</a>",
            escape(url),
            svg(icon::LINK, "icon-sm")
        ));
    }
    if let Some(card) = &card {
        let href = links
            .social_path(card.address())
            .or_else(|| links.repo_path(card.address()));
        if let Some(href) = href {
            buttons.push_str(&format!(
                "<a class=\"btn\" href=\"{}\">{}Source: {}</a>",
                escape(&href),
                svg(icon::REPO, "icon-sm"),
                escape(&card.title())
            ));
        }
    } else if let Some(url) = app.repository() {
        buttons.push_str(&format!(
            "<a class=\"btn\" href=\"{}\" rel=\"nofollow noopener noreferrer\">{}Source code</a>",
            escape(url),
            svg(icon::CODE, "icon-sm")
        ));
    }
    if !buttons.is_empty() {
        body.push_str(&format!("<div class=\"actions-row\">{buttons}</div>"));
    }
    let images = app.images();
    if !images.is_empty() {
        body.push_str("<div class=\"shots\">");
        for url in images.iter().take(6) {
            body.push_str(&format!(
                "<img src=\"{}\" alt=\"A screenshot of {}\" loading=\"lazy\" decoding=\"async\" referrerpolicy=\"no-referrer\">",
                escape(url),
                escape(app.name())
            ));
        }
        body.push_str("</div>");
    }
    if !app.event.content.trim().is_empty() {
        body.push_str(&format!(
            "<div class=\"markdown\" data-description>{}</div>",
            ui_markdown::to_html(&ui_markdown::parse(
                &app.event.content,
                ui_markdown::absolute_only
            ))
        ));
    }

    body.push_str(&release_section(
        &app,
        &all_channels,
        channel.as_deref(),
        &shown,
        &assets,
        card.as_ref().map(RepoCard::address),
        &links,
        &lens,
    ));
    if let Some(card) = &card {
        body.push_str("<h2 class=\"section-title\">Source</h2>");
        body.push_str(&card.render(&links));
    }
    body.push_str("</div><aside class=\"sidebar\">");
    body.push_str(&vouch_card(
        &app,
        &vouched,
        names,
        viewer.as_ref(),
        &found_reports,
        &targets,
        follows.as_ref(),
        card.as_ref(),
    ));
    body.push_str("<div class=\"side-card\"><h2>In app sets</h2>");
    if sets.is_empty() {
        body.push_str("<p class=\"muted small\">No app set lists it yet.</p>");
    } else {
        body.push_str("<ul class=\"plain\">");
        for set in &sets {
            let Some(set_address) = Address::of(set) else {
                continue;
            };
            body.push_str(&format!(
                "<li><a href=\"{PREFIX}/apps/sets/{}\">{}</a> <span class=\"muted small\">by {}</span></li>",
                set_address.naddr(),
                escape(&set_title(set)),
                name(names, &set.pubkey)
            ));
        }
        body.push_str("</ul>");
    }
    body.push_str(
        "<p class=\"muted small\">Curated sets of apps (NIP-51 kind 30267), Zapstore's \
         stacks.</p></div>",
    );
    body.push_str(&details_card(&app, &shown, &assets));
    body.push_str(&tips::split_card(&app.event, names, "the publisher"));
    body.push_str(&format!(
        "<div class=\"side-card\"><h2>Read as</h2>{}<p class=\"muted small\">Count only the people a \
         key follows (its kind 3 list), and show their reports.</p></div>",
        viewer::as_form(viewer.as_ref(), "Viewing as")
    ));
    body.push_str("</aside></div>");
    html(
        pages,
        crate::Nav::Apps,
        StatusCode::OK,
        app.name(),
        None,
        &body,
    )
}

/// The release part of an app page: the channel links, the shown
/// channel's newest release with its notes and files, and the older ones.
#[allow(clippy::too_many_arguments)]
fn release_section(
    app: &App,
    all_channels: &[String],
    channel: Option<&str>,
    shown: &[&Release],
    assets: &HashMap<EventId, Event>,
    repo: Option<&RepoAddress>,
    links: &Links,
    lens: &str,
) -> String {
    let Some((release, older)) = shown.split_first() else {
        return "<h2 class=\"section-title\">Releases</h2><div class=\"box\"><div class=\"empty\">\
                No release published yet.</div></div>"
            .to_owned();
    };
    let mut out = format!(
        "<div class=\"section-head\"><h2 class=\"section-title\" data-release=\"{}\">Release {}</h2>",
        escape(release.version()),
        escape(release.version())
    );
    if all_channels.len() > 1 {
        out.push_str("<nav class=\"seg-links\" aria-label=\"Channel\">");
        for each in all_channels {
            let current = Some(each.as_str()) == channel;
            out.push_str(&format!(
                "<a class=\"seg{}\" href=\"{}?channel={}{lens}\"{}>{}</a>",
                if current { " current" } else { "" },
                escape(&app.path()),
                percent_encode(each),
                if current {
                    " aria-current=\"page\""
                } else {
                    ""
                },
                escape(each)
            ));
        }
        out.push_str("</nav>");
    }
    out.push_str(&format!(
        "</div><p class=\"muted small\">{} channel &middot; released {}</p>",
        escape(release.channel()),
        ago(release.at())
    ));
    if !release.event.content.trim().is_empty() {
        out.push_str(&format!(
            "<div class=\"markdown\" data-notes>{}</div>",
            ui_markdown::to_html(&ui_markdown::parse(
                &release.event.content,
                ui_markdown::absolute_only
            ))
        ));
    }
    out.push_str(&assets_table(release, assets, repo, links));
    if !older.is_empty() {
        out.push_str("<details class=\"older\"><summary>Earlier releases on this channel</summary><ul class=\"plain\">");
        for each in older.iter().take(20) {
            out.push_str(&format!(
                "<li><span class=\"mono\">{}</span> <span class=\"muted small\">{}</span></li>",
                escape(each.version()),
                ago(each.at())
            ));
        }
        out.push_str("</ul></details>");
    }
    out
}

/// A release's files: platform, type, size, hash, commit and where to get
/// it.
fn assets_table(
    release: &Release,
    assets: &HashMap<EventId, Event>,
    repo: Option<&RepoAddress>,
    links: &Links,
) -> String {
    let rows: Vec<&Event> = release
        .asset_ids()
        .iter()
        .filter_map(|id| assets.get(id))
        .collect();
    if rows.is_empty() {
        return "<p class=\"muted small\">This release names no file this relay holds.</p>"
            .to_owned();
    }
    let commit_base = repo.and_then(|repo| links.repo_path(repo));
    let mut out = String::from(
        "<div class=\"box table-wrap\"><table class=\"data-table\" data-assets><thead><tr>\
         <th>Platform</th><th>Format</th><th>Size</th><th>SHA-256</th><th>Built from</th><th>Get it</th>\
         </tr></thead><tbody>",
    );
    for asset in rows {
        let mime = first_tag(asset, "m").map(str::trim).unwrap_or("");
        let platforms = all_values(asset, "f");
        let mut platform = if platforms.is_empty() {
            format!(
                "<strong>{}</strong>",
                escape(mime_platform(mime).unwrap_or("Any"))
            )
        } else {
            platforms
                .iter()
                .map(|id| {
                    format!(
                        "<strong title=\"{}\">{}</strong>",
                        escape(id),
                        escape(&platform_label(id))
                    )
                })
                .collect::<Vec<_>>()
                .join("<br>")
        };
        if let Some(variant) = first_tag(asset, "variant") {
            platform.push_str(&format!(
                " <span class=\"muted small\">{}</span>",
                escape(variant)
            ));
        }
        let size = first_tag(asset, "size")
            .and_then(|size| size.trim().parse::<u64>().ok())
            .map(human_size)
            .unwrap_or_else(|| "—".to_owned());
        let hash = first_tag(asset, "x").map(str::trim).unwrap_or("");
        let hash_html = match short_hash(hash) {
            Some(short) => format!(
                "<span class=\"mono\" title=\"{}\">{short}</span>",
                escape(hash)
            ),
            None => "<span class=\"muted\">none</span>".to_owned(),
        };
        let commit_html = match first_tag(asset, "commit").and_then(commit_id) {
            Some(commit) => match &commit_base {
                Some(base) => format!(
                    "<a class=\"mono\" href=\"{}/commit/{commit}\" title=\"{commit}\">{}</a>",
                    escape(base),
                    &commit[..7]
                ),
                None => format!(
                    "<span class=\"mono\" title=\"{commit}\">{}</span>",
                    &commit[..7]
                ),
            },
            None => "<span class=\"muted\">—</span>".to_owned(),
        };
        let get = match first_tag(asset, "url").and_then(https) {
            Some(url) => format!(
                "<a href=\"{}\" rel=\"nofollow noopener noreferrer\">Download</a>",
                escape(url)
            ),
            None => "<span class=\"muted small\">Fetch by hash from Blossom</span>".to_owned(),
        };
        out.push_str(&format!(
            "<tr data-asset=\"{}\"><td>{platform}</td><td class=\"mono small\">{}</td><td>{size}</td>\
             <td>{hash_html}</td><td>{commit_html}</td><td>{get}</td></tr>",
            asset.id.to_hex(),
            escape(mime),
        ));
    }
    out.push_str(
        "</tbody></table></div><p class=\"muted small\">Each file is pinned by its SHA-256. A file \
         without an <code>https</code> URL is fetched by that hash from a Blossom server; this relay \
         holds no files and fetches none. \"Built from\" is the commit the asset says it came from.</p>",
    );
    out
}

/// "Who vouches": the people, the lens, the reports from them, and who
/// signed the listing.
#[allow(clippy::too_many_arguments)]
fn vouch_card(
    app: &App,
    vouched: &[(PublicKey, Timestamp)],
    names: &HashMap<PublicKey, String>,
    viewer: Option<&PublicKey>,
    found_reports: &[Event],
    targets: &Targets,
    follows: Option<&HashSet<PublicKey>>,
    card: Option<&RepoCard>,
) -> String {
    let mut out = String::from("<div class=\"side-card\" data-vouch><h2>Who vouches for it</h2>");
    if !vouched.is_empty() {
        out.push_str("<div class=\"avatars\">");
        for (key, _) in vouched.iter().take(FACES) {
            out.push_str(&format!(
                "<a href=\"{}\" title=\"{}\">{}</a>",
                relay_ui::chrome::profile_path(key),
                escape(&plain_name(names, key)),
                avatar(&plain_name(names, key), true)
            ));
        }
        out.push_str("</div>");
    }
    let count = vouched.len();
    let line = match viewer {
        Some(viewer) => format!(
            "{count} {} {} follows starred or listed this app.",
            plural(count, "person", "people"),
            name(names, viewer)
        ),
        None => format!(
            "{count} {} starred or listed this app on this relay.",
            plural(count, "person", "people")
        ),
    };
    out.push_str(&format!("<p data-vouchers=\"{count}\">{line}"));
    if let (Some(_), Some(follows)) = (viewer, follows) {
        let from_them = reports::tally(found_reports, targets, Some(follows)).people;
        if from_them == 0 {
            out.push_str(" No reports from them.");
        } else {
            out.push_str(&format!(" {from_them} of them reported it."));
        }
    }
    out.push_str("</p>");
    let author = app.address().author();
    let signer = match card {
        Some(card) if card.is_maker(author) => format!(
            "Signed by {}, who also keeps the source repo.",
            name(names, author)
        ),
        Some(_) => format!(
            "Signed by {}, who is neither the owner nor a maintainer of the repo it links.",
            name(names, author)
        ),
        None => format!("Signed by {}.", name(names, author)),
    };
    out.push_str(&format!(
        "<p class=\"muted small\">{signer} A star is a <code>+</code> reaction on the app's address \
         (<code>a</code> <code>{}</code>); each person counts once.</p></div>",
        escape(&app.address().coordinate())
    ));
    out
}

/// The app's facts: website, source, license, platforms, the NIPs its
/// files say they support, and where the listing comes from.
fn details_card(app: &App, shown: &[&Release], assets: &HashMap<EventId, Event>) -> String {
    let mut out = String::from("<div class=\"side-card\"><h2>Details</h2><ul class=\"facts\">");
    if let Some(url) = app.website() {
        out.push_str(&format!(
            "<li>{}<a class=\"truncate\" href=\"{1}\" rel=\"nofollow noopener noreferrer\">{1}</a></li>",
            svg(icon::LINK, ""),
            escape(url)
        ));
    }
    if let Some(url) = app.repository() {
        out.push_str(&format!(
            "<li>{}<a class=\"truncate\" href=\"{1}\" rel=\"nofollow noopener noreferrer\">{1}</a></li>",
            svg(icon::CODE, ""),
            escape(url)
        ));
    }
    out.push_str("</ul>");
    if let Some(license) = app.license() {
        out.push_str(&format!(
            "<p>License <span class=\"strong\">{}</span></p>",
            escape(license)
        ));
    }
    let platforms = app.platforms();
    if !platforms.is_empty() {
        out.push_str(&format!(
            "<p>Platforms {}</p>",
            platforms
                .iter()
                .map(|id| format!(
                    "<span class=\"chip\" title=\"{}\">{}</span>",
                    escape(id),
                    escape(&platform_label(id))
                ))
                .collect::<String>()
        ));
    }
    let mut nips: Vec<&str> = Vec::new();
    if let Some(release) = shown.first() {
        for id in release.asset_ids() {
            for nip in assets
                .get(&id)
                .map(|asset| all_values(asset, "supported_nip"))
                .unwrap_or_default()
            {
                if !nips.contains(&nip) {
                    nips.push(nip);
                }
            }
        }
    }
    if !nips.is_empty() {
        out.push_str(&format!(
            "<p>Supports {}</p>",
            nips.iter()
                .map(|nip| format!("NIP-{}", escape(nip)))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    out.push_str(&format!(
        "<p class=\"muted small\">From a kind 32267 app event. {DRAFT_NOTE}</p></div>"
    ));
    out
}

/// `/social/apps/sets/<naddr>`: one app set and the apps it lists.
pub(crate) async fn set(State(pages): State<Pages>, Path(target): Path<String>) -> Response {
    let Some(address) = Address::from_naddr(&target, APP_SET) else {
        return not_found(&pages);
    };
    let store = &pages.store;
    let events = store
        .query(
            Filter::new()
                .kind(Kind::from(APP_SET))
                .author(*address.author())
                .identifier(address.d()),
        )
        .await;
    let Some(set) = newest(events.iter().filter(|event| {
        event.pubkey == *address.author() && ui_nip34::repo::d_tag(event) == address.d()
    }))
    .cloned() else {
        return not_found(&pages);
    };
    let listed = Address::tagged(&set, SOFTWARE_APP);
    let mut authors: Vec<PublicKey> = Vec::new();
    for each in &listed {
        if !authors.contains(each.author()) {
            authors.push(*each.author());
        }
    }
    let apps: HashMap<Address, App> = if authors.is_empty() {
        HashMap::new()
    } else {
        latest_apps(
            store
                .query(
                    Filter::new()
                        .kind(Kind::from(SOFTWARE_APP))
                        .authors(authors.clone())
                        .limit(LIMIT),
                )
                .await,
        )
        .into_iter()
        .map(|app| (app.address().clone(), app))
        .collect()
    };
    let mut keys = authors;
    if !keys.contains(&set.pubkey) {
        keys.push(set.pubkey);
    }
    let profiles = Profiles::load(store, &keys).await;
    let names = profiles.names();
    let title = set_title(&set);
    let mut body = format!(
        "<h1 class=\"page-title\">{}</h1><p class=\"muted small\">An app set (NIP-51 kind 30267) by {} \
         &middot; updated {}</p>",
        escape(&title),
        name(names, &set.pubkey),
        ago(set.created_at)
    );
    if let Some(description) = first_tag(&set, "description") {
        body.push_str(&format!("<p>{}</p>", escape(description)));
    }
    let shown: Vec<&App> = listed.iter().filter_map(|each| apps.get(each)).collect();
    if shown.is_empty() {
        body.push_str("<div class=\"box\"><div class=\"empty\">None of the apps this set lists are on this relay.</div></div>");
    } else {
        body.push_str("<div class=\"app-grid\">");
        for app in &shown {
            body.push_str(&tile(app, names, None));
        }
        body.push_str("</div>");
    }
    if shown.len() < listed.len() {
        body.push_str(&format!(
            "<p class=\"muted small\">{} more {} not on this relay.</p>",
            listed.len() - shown.len(),
            plural(listed.len() - shown.len(), "app is", "apps are")
        ));
    }
    body.push_str(&format!("<p><a href=\"{PREFIX}/apps\">All apps</a></p>"));
    html(
        &pages,
        crate::Nav::Apps,
        StatusCode::OK,
        &title,
        None,
        &body,
    )
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;

    use super::*;

    fn signed(keys: &Keys, kind: u16, at: u64, content: &str, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(kind), content)
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .custom_created_at(Timestamp::from(at))
            .finalize(keys)
            .unwrap()
    }

    fn release(keys: &Keys, at: u64, d: &str, version: &str, channel: &str) -> Event {
        signed(
            keys,
            SOFTWARE_RELEASE,
            at,
            "",
            &[
                &["i", d],
                &["version", version],
                &["d", &format!("{d}@{version}")],
                &["c", channel],
            ],
        )
    }

    #[test]
    fn an_address_keeps_the_colons_in_its_d() {
        let author = Keys::generate().public_key();
        let coordinate = format!("32267:{}:dev.tide:app", author.to_hex());
        let address = Address::parse(&coordinate, SOFTWARE_APP).unwrap();
        assert_eq!(address.d(), "dev.tide:app");
        assert_eq!(address.coordinate(), coordinate);
        assert_eq!(
            Address::from_naddr(&address.naddr(), SOFTWARE_APP),
            Some(address.clone())
        );
        assert_eq!(Address::from_naddr(&address.naddr(), APP_SET), None);
        assert_eq!(Address::parse(&coordinate, APP_SET), None);
        assert_eq!(Address::parse("32267:nope:x", SOFTWARE_APP), None);
    }

    #[test]
    fn the_newest_release_on_a_channel_is_the_highest_version_not_the_latest_date() {
        let maker = Keys::generate();
        let app = Address::new(SOFTWARE_APP, maker.public_key(), "dev.tide.tidepool").unwrap();
        let events = vec![
            release(&maker, 30, "dev.tide.tidepool", "1.1.9", "main"),
            release(&maker, 10, "dev.tide.tidepool", "v1.2.0", "main"),
            release(&maker, 20, "dev.tide.tidepool", "1.3.0-beta.1", "beta"),
            release(&maker, 40, "dev.tide.tidepool", "0.0.1", "lts"),
            // Another app's release, and a stranger's release of this one.
            release(&maker, 50, "other.app", "9.0.0", "main"),
            release(&Keys::generate(), 60, "dev.tide.tidepool", "9.9.9", "main"),
        ];
        let releases = releases_of(&app, events);
        assert_eq!(releases.len(), 4);
        assert_eq!(channels(&releases), ["main", "beta", "lts"]);
        let main: Vec<&str> = on_channel(&releases, "main")
            .iter()
            .map(|release| release.version())
            .collect();
        assert_eq!(main, ["v1.2.0", "1.1.9"]);
        assert_eq!(latest_release(&releases).unwrap().version(), "v1.2.0");
    }

    #[test]
    fn a_replaced_release_counts_once() {
        let maker = Keys::generate();
        let app = Address::new(SOFTWARE_APP, maker.public_key(), "a").unwrap();
        let events = vec![
            release(&maker, 10, "a", "1.0.0", "main"),
            release(&maker, 20, "a", "1.0.0", "beta"),
        ];
        let releases = releases_of(&app, events);
        assert_eq!(releases.len(), 1);
        assert_eq!(releases[0].channel(), "beta");
    }

    #[test]
    fn people_who_vouch_are_counted_once_and_may_be_limited_to_follows() {
        let maker = Keys::generate();
        let coordinate = format!("32267:{}:a", maker.public_key().to_hex());
        let (fan, curator, stranger) = (Keys::generate(), Keys::generate(), Keys::generate());
        let events = vec![
            signed(
                &fan,
                REACTION,
                1,
                "+",
                &[&["a", &coordinate], &["k", "32267"]],
            ),
            signed(&fan, REACTION, 2, "+", &[&["a", &coordinate]]),
            signed(
                &curator,
                APP_SET,
                3,
                "",
                &[&["d", "kit"], &["a", &coordinate]],
            ),
            signed(&stranger, REACTION, 4, "-", &[&["a", &coordinate]]),
            signed(
                &stranger,
                REACTION,
                5,
                "+",
                &[&["a", &coordinate], &["k", "30617"]],
            ),
        ];
        let all = vouchers(&events, &coordinate, None);
        assert_eq!(all.len(), 2);
        let follows: HashSet<PublicKey> = [curator.public_key()].into();
        assert_eq!(
            vouchers(&events, &coordinate, Some(&follows)),
            vec![(curator.public_key(), Timestamp::from(3))]
        );
    }

    #[test]
    fn sizes_hashes_and_platforms_read_as_people_say_them() {
        assert_eq!(human_size(999), "999 B");
        assert_eq!(human_size(8_100_000), "8.1 MB");
        assert_eq!(human_size(1_300_000), "1.3 MB");
        assert_eq!(human_size(14_600_000_000), "14.6 GB");
        let hash = format!("3f9a{}c21e", "0".repeat(56));
        assert_eq!(short_hash(&hash).as_deref(), Some("3f9a…c21e"));
        assert_eq!(short_hash("not-hex-at-all!"), None);
        assert_eq!(platform_label("linux-x86_64"), "Linux x86_64");
        assert_eq!(platform_label("darwin-arm64"), "macOS arm64");
        assert_eq!(platform_label("wasm32"), "WebAssembly 32");
        assert_eq!(
            mime_platform("application/vnd.android.package-archive"),
            Some("Android")
        );
        assert_eq!(commit_id(&"a".repeat(40)), Some("a".repeat(40).as_str()));
        assert_eq!(commit_id("a1b2c3d"), None);
    }

    #[test]
    fn an_app_shows_only_https_images_and_its_newest_event() {
        let maker = Keys::generate();
        let old = signed(
            &maker,
            SOFTWARE_APP,
            1,
            "",
            &[&["d", "a"], &["name", "Old"]],
        );
        let new = signed(
            &maker,
            SOFTWARE_APP,
            2,
            "",
            &[
                &["d", "a"],
                &["name", "New"],
                &["icon", "http://cdn.test/icon.png"],
                &["image", "https://cdn.test/1.png"],
                &["image", "javascript:alert(1)"],
                &["image", "https://cdn.test/1.png"],
            ],
        );
        let apps = latest_apps(vec![old, new]);
        assert_eq!(apps.len(), 1);
        assert_eq!(apps[0].name(), "New");
        assert_eq!(apps[0].icon(), None);
        assert_eq!(apps[0].images(), ["https://cdn.test/1.png"]);
    }
}
