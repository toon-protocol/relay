//! A repo's live demo (proposal 8 of `docs/social-for-builders-ux.md`):
//! the one rule every page uses to put a "Live demo" button on a repo card,
//! the repo page and a launch.
//!
//! A demo is the maker's (rule 2 of §7.0): only an event signed by an
//! authorized author of the repo (ADR 0002) can name one. In order:
//!
//! 1. a launch that is an app event (kind 31990) names its NIP-5A site in an
//!    `a` tag, and that site is signed by an authorized author;
//! 2. else the newest NIP-5A site manifest (kind 35128, or the root site
//!    15128) with at least one `path` tag whose `source` tag is a `nostr://`
//!    clone URL of the repo, in any of NIP-34's three forms;
//! 3. else the newest app event about the repo (`a` tag naming it): a kind
//!    31990 naming a site, as in 1, or a kind 32267 software application
//!    (nips#1336, an open PR) with an `https` `url` tag.
//!
//! A site is linked through the operator's nsite gateway
//! (`TOON_UI_NSITE_GATEWAY`), and without one a site is no demo; a 32267's
//! `url` is linked as it is. Either way the relay serves and fetches nothing
//! of the demo (ADR 0001).

use std::collections::HashMap;

use nostr::event::{Event, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use relay_ui::ReadOnlyStore;
use ui_nip34::card::RepoCard;
use ui_nip34::repo::{RepoAddress, first_tag, newest};

use crate::kinds::{APP_HANDLER, SITE_NAMED, SITE_ROOT, SOFTWARE_APP};
use crate::ship::{self, Site};

/// The most site manifests or app events one repo's demo is looked for in.
const SCAN: usize = 200;

/// A live demo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Demo {
    /// A NIP-5A site, reached through the gateway.
    Site(Site),
    /// A web app's own `https` URL (a kind 32267's `url`).
    Web(String),
}

impl Demo {
    /// Where the button goes, if it can go anywhere: a site needs a gateway.
    pub(crate) fn url(&self, gateway: Option<&str>) -> Option<String> {
        match self {
            Self::Site(site) => site.url(gateway?),
            Self::Web(url) => Some(url.clone()),
        }
    }
}

/// Whether `event` is a NIP-5A site manifest: kind 35128 or 15128 with at
/// least one `path` tag, which the NIP requires. A pure function.
pub(crate) fn is_manifest(event: &Event) -> bool {
    matches!(event.kind.as_u16(), SITE_NAMED | SITE_ROOT)
        && event.tags.iter().map(|tag| tag.as_slice()).any(
            |tag| matches!(tag, [name, path, _hash, ..] if name == "path" && path.starts_with('/')),
        )
}

/// Whether site manifest `site` says its source is `card`'s repo.
pub(crate) fn sourced_from(site: &Event, card: &RepoCard) -> bool {
    let nip05 = card.profiles().nip05(card.address().owner());
    site.tags.iter().map(|tag| tag.as_slice()).any(|tag| {
        matches!(tag, [name, value, ..] if name == "source" && ship::source_names(value, card.address(), nip05))
    })
}

/// The demo an app event (a launch) names: its site, if an authorized
/// author of the repo signed it. A pure function.
pub(crate) fn of_app_event(event: &Event, card: &RepoCard) -> Option<Demo> {
    if !card.is_maker(&event.pubkey) {
        return None;
    }
    ship::app_site(event)
        .filter(|site| card.is_maker(site.author()))
        .map(Demo::Site)
}

/// The demo an app event about the repo names: a 31990's site (as
/// [`of_app_event`]) or a 32267's `https` `url`. A pure function.
fn of_app(event: &Event, card: &RepoCard) -> Option<Demo> {
    if !card.is_maker(&event.pubkey) || !RepoAddress::tagged(event, "a").contains(card.address()) {
        return None;
    }
    match event.kind.as_u16() {
        APP_HANDLER => of_app_event(event, card),
        SOFTWARE_APP => first_tag(event, "url")
            .map(str::trim)
            .filter(|url| url.starts_with("https://") && !url.contains(char::is_whitespace))
            .map(|url| Demo::Web(url.to_owned())),
        _ => None,
    }
}

/// The demo among `sites` and `apps` for `card`'s repo, by the module's
/// order, that can be linked with `gateway`. A pure function.
pub(crate) fn choose(
    card: &RepoCard,
    sites: &[Event],
    apps: &[Event],
    gateway: Option<&str>,
) -> Option<Demo> {
    let linkable = |demo: &Demo| demo.url(gateway).is_some();
    let site = newest(sites.iter().filter(|site| {
        card.is_maker(&site.pubkey) && is_manifest(site) && sourced_from(site, card)
    }))
    .and_then(Site::of)
    .map(Demo::Site)
    .filter(linkable);
    if site.is_some() {
        return site;
    }
    let mut apps: Vec<&Event> = apps.iter().collect();
    apps.sort_by(|a, b| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
    apps.into_iter()
        .filter_map(|app| of_app(app, card))
        .find(linkable)
}

/// The site manifests `card`'s authorized authors signed whose `source` is
/// the repo, newest first.
pub(crate) async fn maker_sites(store: &ReadOnlyStore, card: &RepoCard) -> Vec<Event> {
    let mut sites: Vec<Event> = store
        .query(
            Filter::new()
                .kinds([Kind::from(SITE_NAMED), Kind::from(SITE_ROOT)])
                .authors(card.authorized().to_vec())
                .limit(SCAN),
        )
        .await
        .into_iter()
        .filter(|site| card.is_maker(&site.pubkey) && is_manifest(site) && sourced_from(site, card))
        .collect();
    sites.sort_by(|a, b| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
    sites
}

/// `card`'s repo's live demo, linkable with `gateway`, if it has one.
pub(crate) async fn load(
    store: &ReadOnlyStore,
    card: &RepoCard,
    gateway: Option<&str>,
) -> Option<Demo> {
    let sites = if gateway.is_some() {
        maker_sites(store, card).await
    } else {
        Vec::new()
    };
    let apps = store
        .query(
            Filter::new()
                .kinds([Kind::from(APP_HANDLER), Kind::from(SOFTWARE_APP)])
                .authors(card.authorized().to_vec())
                .custom_tag(SingleLetterTag::LOWERCASE_A, card.address().coordinate())
                .limit(SCAN),
        )
        .await;
    choose(card, &sites, &apps, gateway)
}

/// The demos of the repos in `cards`, for a page that draws them: a launch
/// among `launches` that is an app event names its own site first.
pub(crate) async fn for_cards(
    store: &ReadOnlyStore,
    cards: &HashMap<RepoAddress, RepoCard>,
    launches: &[(&Event, RepoAddress)],
    gateway: Option<&str>,
) -> HashMap<RepoAddress, Demo> {
    let mut demos = HashMap::new();
    for (event, repo) in launches {
        if let Some(card) = cards.get(repo)
            && let Some(demo) = of_app_event(event, card)
            && demo.url(gateway).is_some()
        {
            demos.entry(repo.clone()).or_insert(demo);
        }
    }
    for (repo, card) in cards {
        if demos.contains_key(repo) {
            continue;
        }
        if let Some(demo) = load(store, card, gateway).await {
            demos.insert(repo.clone(), demo);
        }
    }
    demos
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

    fn card_of(owner: &Keys, d: &str) -> RepoCard {
        let announcement = signed(owner, 30_617, 1, &[&["d", d]]);
        RepoCard::from_announcement(&announcement).unwrap()
    }

    #[test]
    fn a_maker_site_whose_source_is_the_repo_is_its_demo() {
        let maker = Keys::generate();
        let stranger = Keys::generate();
        let card = card_of(&maker, "tidepool");
        let source = format!("nostr://{}/relay.test/tidepool", npub(&maker.public_key()));
        let path: &[&str] = &["path", "/index.html", &"f".repeat(64)];
        let site = signed(
            &maker,
            35_128,
            2,
            &[&["d", "tidepool"], path, &["source", &source]],
        );
        let gateway = Some("https://nsite.test");
        assert_eq!(
            choose(&card, std::slice::from_ref(&site), &[], gateway),
            Site::of(&site).map(Demo::Site)
        );
        // Without a gateway, a site is no demo.
        assert_eq!(choose(&card, std::slice::from_ref(&site), &[], None), None);
        // A stranger's site naming the repo, or a manifest with no files, is not.
        let copy = signed(
            &stranger,
            35_128,
            3,
            &[&["d", "tidepool"], path, &["source", &source]],
        );
        let empty = signed(&maker, 35_128, 4, &[&["d", "empty"], &["source", &source]]);
        assert_eq!(choose(&card, &[copy, empty], &[], gateway), None);
    }

    #[test]
    fn an_app_listing_with_an_https_url_is_a_demo_without_a_gateway() {
        let maker = Keys::generate();
        let card = card_of(&maker, "gitsail");
        let a = card.address().coordinate();
        let app = signed(
            &maker,
            32_267,
            2,
            &[
                &["d", "dev.gitsail"],
                &["a", &a],
                &["url", "https://gitsail.test"],
            ],
        );
        assert_eq!(
            choose(&card, &[], std::slice::from_ref(&app), None),
            Some(Demo::Web("https://gitsail.test".to_owned()))
        );
        let plain = signed(
            &maker,
            32_267,
            3,
            &[&["a", &a], &["url", "http://gitsail.test"]],
        );
        let elsewhere = signed(&maker, 32_267, 4, &[&["url", "https://other.test"]]);
        assert_eq!(choose(&card, &[], &[plain, elsewhere], None), None);
    }

    #[test]
    fn an_app_event_names_its_site_only_if_a_maker_signed_the_site() {
        let maker = Keys::generate();
        let stranger = Keys::generate();
        let card = card_of(&maker, "surfcast");
        let a = card.address().coordinate();
        let own = format!("35128:{}:surfcast", maker.public_key().to_hex());
        let theirs = format!("35128:{}:surfcast", stranger.public_key().to_hex());
        let app = signed(&maker, 31_990, 2, &[&["a", &a], &["a", &own]]);
        assert!(matches!(of_app_event(&app, &card), Some(Demo::Site(_))));
        let borrowed = signed(&maker, 31_990, 2, &[&["a", &a], &["a", &theirs]]);
        assert_eq!(of_app_event(&borrowed, &card), None);
    }
}
