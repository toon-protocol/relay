//! "Open in…" (proposal 11 of `docs/social-for-builders-ux.md`): the apps
//! people recommend for repositories, by NIP-89 (`89.md`, verified at
//! nostr-protocol/nips@d47a2ec).
//!
//! A handler is a kind 31990 that declares `k` `30617` and gives a `web` URL
//! template for an `naddr` (a `web` tag marked `naddr`, else one with no
//! entity type, which NIP-89 makes the generic handler). Its `<bech32>`
//! placeholder becomes the repo's `naddr`. Only `https` templates are drawn.
//!
//! NIP-89 warns that any key can publish a handler, so a page lists only
//! handlers someone recommends: a kind 31989 with `d` `30617` whose `a` tag
//! names the handler. Handlers are ranked by the people who recommend them,
//! counted once each; when the page has a viewer (`?as=<npub>`), the people
//! that viewer follows (kind 3 and kind 10017 `p` tags) and the viewer come
//! first, as NIP-89's user flow queries "the user's and the user's follows".
//!
//! A **Remix** link is drawn only for a handler this module knows can clone
//! a `nostr://` repository: one whose `web` template is on a host in
//! [`CLONERS`]. NIP-89 has no way to say "I can clone a repo", so the list
//! is code, each entry with the source that shows it; a handler not on it
//! gets no Remix link, however it is recommended.

use std::collections::{HashMap, HashSet};

use nostr::event::{Event, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use nostr::nips::nip01::Coordinate;
use nostr::nips::nip19::{Nip19Coordinate, ToBech32};
use nostr::types::RelayUrl;
use relay_ui::ReadOnlyStore;
use relay_ui::chrome::{Profiles, escape, percent_encode};
use ui_nip34::repo::{ANNOUNCEMENT, RepoAddress, d_tag, newest};

use crate::kinds::{APP_HANDLER, FOLLOWS, GIT_AUTHORS, HANDLER_RECOMMENDATION};
use crate::ship;

/// App builders known to clone a `nostr://` repository, by the host of
/// their handler's `web` template, and the URL a clone is opened at (the
/// `nostr://` clone URL, percent-encoded, follows it).
///
/// - Shakespeare's `/clone` page takes `?url=` with a `nostr://` URI
///   (`soapbox-pub/shakespeare`, `src/pages/Clone.tsx`, the line the
///   research note §5.4 cites).
pub(crate) const CLONERS: &[(&str, &str)] =
    &[("shakespeare.diy", "https://shakespeare.diy/clone?url=")];

/// The most handlers, and recommendations, one page reads.
const SCAN: usize = 500;
/// The most handlers the sidebar shows.
const SHOWN: usize = 6;

/// The `web` template `handler` gives for an `naddr`: a `web` tag marked
/// `naddr`, else a generic one (no entity type), if it is `https` and holds
/// the `<bech32>` placeholder. A pure function.
pub(crate) fn naddr_template(handler: &Event) -> Option<&str> {
    let webs: Vec<&[String]> = handler
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(|tag| tag.first().is_some_and(|name| name == "web"))
        .collect();
    let usable = |url: &&String| url.starts_with("https://") && has_placeholder(url);
    webs.iter()
        .find_map(|tag| match tag {
            [_, url, kind, ..] if kind == "naddr" => Some(url).filter(usable),
            _ => None,
        })
        .or_else(|| {
            webs.iter().find_map(|tag| match tag {
                [_, url] => Some(url).filter(usable),
                [_, url, kind, ..] if kind.is_empty() => Some(url).filter(usable),
                _ => None,
            })
        })
        .map(String::as_str)
}

/// Whether a template names the entity to open. NIP-89 writes `<bech32>`;
/// `<nip19>` is read as the same placeholder.
fn has_placeholder(template: &str) -> bool {
    template.contains("<bech32>") || template.contains("<nip19>")
}

/// `template` with its placeholder replaced by `entity`. A pure function.
pub(crate) fn fill(template: &str, entity: &str) -> String {
    template
        .replace("<bech32>", entity)
        .replace("<nip19>", entity)
}

/// Whether `handler` declares that it handles repositories (`k` `30617`).
pub(crate) fn handles_repos(handler: &Event) -> bool {
    handler.kind == Kind::from(APP_HANDLER)
        && handler.tags.iter().map(|tag| tag.as_slice()).any(
            |tag| matches!(tag, [name, kind, ..] if name == "k" && *kind == ANNOUNCEMENT.to_string()),
        )
}

/// The host of an `https` URL.
fn host(url: &str) -> Option<&str> {
    url.strip_prefix("https://")?
        .split(['/', '?', '#', ':'])
        .next()
        .filter(|host| !host.is_empty())
}

/// The URL that opens a copy of the repo at `clone_url` (`nostr://…`) in
/// `handler`, if `handler` is a builder in [`CLONERS`]. A pure function.
pub(crate) fn remix_url(handler: &Event, clone_url: &str) -> Option<String> {
    let host = host(naddr_template(handler)?)?;
    CLONERS
        .iter()
        .find(|(known, _)| host.eq_ignore_ascii_case(known))
        .map(|(_, open)| format!("{open}{}", percent_encode(clone_url)))
}

/// The repo's `naddr`, with this relay as its hint when the host is known.
pub(crate) fn naddr(repo: &RepoAddress, relay_host: Option<&str>) -> String {
    let coordinate = Coordinate::new(Kind::from(ANNOUNCEMENT), *repo.owner()).identifier(repo.d());
    let relays: Vec<RelayUrl> = relay_host
        .and_then(|host| RelayUrl::parse(&format!("wss://{host}")).ok())
        .into_iter()
        .collect();
    Nip19Coordinate::new(coordinate, relays)
        .to_bech32()
        .unwrap_or_default()
}

/// A recommended handler, and who recommends it.
#[derive(Debug, Clone)]
pub(crate) struct Choice {
    handler: Event,
    /// Distinct people who recommend it.
    people: usize,
    /// Of those, the viewer and the people the viewer follows.
    followed: usize,
}

impl Choice {
    /// The handler event.
    pub(crate) fn handler(&self) -> &Event {
        &self.handler
    }
}

/// `handlers` that someone among `recommendations` recommends for repos,
/// ranked: by the viewer and the keys in `follows` first, then by everyone,
/// then newest. Each person counts once per handler, from their newest
/// recommendation. A pure function.
pub(crate) fn rank(
    handlers: &[Event],
    recommendations: &[Event],
    follows: &HashSet<PublicKey>,
) -> Vec<Choice> {
    // A recommendation is addressable by (author, d): only the newest counts.
    let mut newest_by: HashMap<PublicKey, &Event> = HashMap::new();
    for event in recommendations.iter().filter(|event| {
        event.kind == Kind::from(HANDLER_RECOMMENDATION) && d_tag(event) == ANNOUNCEMENT.to_string()
    }) {
        match newest_by.get(&event.pubkey) {
            Some(held) if !ui_nip34::repo::supersedes(event, held) => {}
            _ => {
                newest_by.insert(event.pubkey, event);
            }
        }
    }
    let mut by_coordinate: HashMap<String, Vec<&Event>> = HashMap::new();
    for handler in handlers.iter().filter(|handler| handles_repos(handler)) {
        by_coordinate
            .entry(format!(
                "{APP_HANDLER}:{}:{}",
                handler.pubkey.to_hex(),
                d_tag(handler)
            ))
            .or_default()
            .push(handler);
    }
    let mut choices: Vec<Choice> = Vec::new();
    for (coordinate, versions) in by_coordinate {
        let Some(handler) = newest(versions) else {
            continue;
        };
        if naddr_template(handler).is_none() {
            continue;
        }
        let recommenders: Vec<&PublicKey> = newest_by
            .iter()
            .filter(|(_, event)| {
                event.tags.iter().map(|tag| tag.as_slice()).any(
                    |tag| matches!(tag, [name, value, ..] if name == "a" && *value == coordinate),
                )
            })
            .map(|(key, _)| key)
            .collect();
        if recommenders.is_empty() {
            continue;
        }
        choices.push(Choice {
            handler: handler.clone(),
            people: recommenders.len(),
            followed: recommenders
                .iter()
                .filter(|key| follows.contains(key))
                .count(),
        });
    }
    choices.sort_by(|a, b| {
        (b.followed, b.people, b.handler.created_at, a.handler.id).cmp(&(
            a.followed,
            a.people,
            a.handler.created_at,
            b.handler.id,
        ))
    });
    choices
}

/// The viewer and the keys the viewer follows: the `p` tags of the newest
/// kind 3 and kind 10017 the viewer signed. A pure function.
pub(crate) fn followed_by(viewer: &PublicKey, lists: &[Event]) -> HashSet<PublicKey> {
    let mut keys = HashSet::from([*viewer]);
    for kind in [FOLLOWS, GIT_AUTHORS] {
        let list = newest(
            lists
                .iter()
                .filter(|event| event.pubkey == *viewer && event.kind == Kind::from(kind)),
        );
        for tag in list.iter().flat_map(|list| list.tags.iter()) {
            if let [name, key, ..] = tag.as_slice()
                && name == "p"
                && let Ok(key) = PublicKey::from_hex(key)
            {
                keys.insert(key);
            }
        }
    }
    keys
}

/// The handlers recommended for repos on this relay, ranked for `viewer`,
/// with the profiles their names may come from.
pub(crate) async fn load(
    store: &ReadOnlyStore,
    viewer: Option<&PublicKey>,
) -> (Vec<Choice>, Profiles) {
    let recommendations = store
        .query(
            Filter::new()
                .kind(Kind::from(HANDLER_RECOMMENDATION))
                .identifier(ANNOUNCEMENT.to_string())
                .limit(SCAN),
        )
        .await;
    if recommendations.is_empty() {
        return (Vec::new(), Profiles::default());
    }
    let handlers = store
        .query(
            Filter::new()
                .kind(Kind::from(APP_HANDLER))
                .custom_tag(SingleLetterTag::LOWERCASE_K, ANNOUNCEMENT.to_string())
                .limit(SCAN),
        )
        .await;
    let follows = match viewer {
        Some(viewer) => {
            let lists = store
                .query(
                    Filter::new()
                        .kinds([Kind::from(FOLLOWS), Kind::from(GIT_AUTHORS)])
                        .author(*viewer),
                )
                .await;
            followed_by(viewer, &lists)
        }
        None => HashSet::new(),
    };
    let mut choices = rank(&handlers, &recommendations, &follows);
    choices.truncate(SHOWN);
    let keys: Vec<PublicKey> = choices.iter().map(|choice| choice.handler.pubkey).collect();
    let profiles = Profiles::load(store, &keys).await;
    (choices, profiles)
}

/// What a handler is called: its metadata's name, else its key's profile
/// name, else its template's host.
pub(crate) fn label(handler: &Event, profiles: &Profiles) -> String {
    ship::app_metadata(handler)
        .0
        .or_else(|| profiles.names().get(&handler.pubkey).cloned())
        .or_else(|| naddr_template(handler).and_then(host).map(str::to_owned))
        .unwrap_or_else(|| "an app".to_owned())
}

/// The sidebar's "Open in…" card for `repo`.
pub(crate) fn open_in_card(
    choices: &[Choice],
    profiles: &Profiles,
    repo: &RepoAddress,
    relay_host: Option<&str>,
    viewer: bool,
) -> String {
    let mut out = String::from("<div class=\"side-card\" data-open-in><h2>Open in…</h2>");
    if choices.is_empty() {
        out.push_str(
            "<p class=\"muted small\">No app is recommended for repositories here yet. An app \
             announces itself with a kind 31990 that has <code>k</code> <code>30617</code>; \
             anyone recommends it with a kind 31989 whose <code>d</code> is <code>30617</code> \
             (NIP-89).</p></div>",
        );
        return out;
    }
    out.push_str(&format!(
        "<p class=\"muted small\">Apps people here recommend for repositories (NIP-89){}.</p>\
         <div class=\"links\">",
        if viewer {
            ", the ones the people you follow recommend first"
        } else {
            ""
        }
    ));
    let entity = naddr(repo, relay_host);
    for choice in choices {
        let Some(template) = naddr_template(&choice.handler) else {
            continue;
        };
        let mut title = format!(
            "Recommended by {} {}",
            choice.people,
            ui_nip34::card::plural(choice.people, "person", "people")
        );
        if viewer {
            title.push_str(&format!(", {} you follow", choice.followed));
        }
        out.push_str(&format!(
            "<a class=\"btn\" href=\"{}\" rel=\"noopener noreferrer\" title=\"{}\" data-handler=\"{}:{}:{}\">{}</a>",
            escape(&fill(template, &entity)),
            escape(&title),
            APP_HANDLER,
            choice.handler.pubkey.to_hex(),
            escape(d_tag(&choice.handler)),
            escape(&label(&choice.handler, profiles)),
        ));
    }
    out.push_str("</div></div>");
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

    fn handler(keys: &Keys, d: &str, web: &[&str]) -> Event {
        let mut tags: Vec<&[&str]> = vec![&["k", "30617"]];
        tags.push(web);
        let d_tag: &[&str] = &["d", d];
        tags.push(d_tag);
        signed(keys, 31_990, 1, &tags)
    }

    fn coordinate(handler: &Event) -> String {
        format!("31990:{}:{}", handler.pubkey.to_hex(), d_tag(handler))
    }

    #[test]
    fn the_naddr_template_is_preferred_over_the_generic_one() {
        let app = Keys::generate();
        let both = signed(
            &app,
            31_990,
            1,
            &[
                &["web", "https://app.test/e/<bech32>", "nevent"],
                &["web", "https://app.test/any/<bech32>"],
                &["web", "https://app.test/r/<bech32>", "naddr"],
            ],
        );
        assert_eq!(naddr_template(&both), Some("https://app.test/r/<bech32>"));
        let generic = signed(&app, 31_990, 1, &[&["web", "https://app.test/<nip19>"]]);
        assert_eq!(naddr_template(&generic), Some("https://app.test/<nip19>"));
        let unsafe_scheme = signed(&app, 31_990, 1, &[&["web", "http://app.test/<bech32>"]]);
        assert_eq!(naddr_template(&unsafe_scheme), None);
        let no_placeholder = signed(&app, 31_990, 1, &[&["web", "https://app.test/"]]);
        assert_eq!(naddr_template(&no_placeholder), None);
        assert_eq!(
            fill("https://app.test/r/<bech32>", "naddr1x"),
            "https://app.test/r/naddr1x"
        );
    }

    #[test]
    fn only_recommended_handlers_are_listed_and_the_viewers_follows_come_first() {
        let (workshop, builder, spam) = (Keys::generate(), Keys::generate(), Keys::generate());
        let (dan, ruth, maya) = (Keys::generate(), Keys::generate(), Keys::generate());
        let gitworkshop = handler(
            &workshop,
            "gw",
            &["web", "https://gitworkshop.test/<bech32>", "naddr"],
        );
        let shakespeare = handler(&builder, "sk", &["web", "https://shakespeare.diy/<bech32>"]);
        let unrecommended = handler(&spam, "x", &["web", "https://spam.test/<bech32>"]);
        let not_for_repos = signed(
            &spam,
            31_990,
            1,
            &[&["k", "1"], &["web", "https://spam.test/<bech32>"]],
        );
        let recommend = |who: &Keys, at: u64, of: &Event| {
            signed(
                who,
                31_989,
                at,
                &[&["d", "30617"], &["a", &coordinate(of), "", "web"]],
            )
        };
        let recommendations = vec![
            recommend(&dan, 1, &gitworkshop),
            recommend(&ruth, 1, &gitworkshop),
            recommend(&maya, 2, &shakespeare),
            signed(
                &maya,
                31_989,
                1,
                &[&["d", "1"], &["a", &coordinate(&not_for_repos)]],
            ),
        ];
        let handlers = vec![
            gitworkshop.clone(),
            shakespeare.clone(),
            unrecommended,
            not_for_repos,
        ];

        let anyone = rank(&handlers, &recommendations, &HashSet::new());
        assert_eq!(anyone.len(), 2);
        assert_eq!(
            anyone[0].handler().id,
            gitworkshop.id,
            "two people beat one"
        );
        assert_eq!(anyone[0].people, 2);

        // Viewing as someone who follows Maya: her pick comes first.
        let follows = HashSet::from([maya.public_key()]);
        let viewed = rank(&handlers, &recommendations, &follows);
        assert_eq!(viewed[0].handler().id, shakespeare.id);
        assert_eq!(viewed[0].followed, 1);
    }

    #[test]
    fn a_newer_recommendation_replaces_the_older() {
        let (workshop, builder, dan) = (Keys::generate(), Keys::generate(), Keys::generate());
        let gitworkshop = handler(
            &workshop,
            "gw",
            &["web", "https://gitworkshop.test/<bech32>"],
        );
        let shakespeare = handler(&builder, "sk", &["web", "https://shakespeare.diy/<bech32>"]);
        let recommend = |at: u64, of: &Event| {
            signed(
                &dan,
                31_989,
                at,
                &[&["d", "30617"], &["a", &coordinate(of)]],
            )
        };
        let older = recommend(1, &gitworkshop);
        let ranked = rank(
            &[gitworkshop, shakespeare.clone()],
            &[older, recommend(2, &shakespeare)],
            &HashSet::new(),
        );
        assert_eq!(ranked.len(), 1, "Dan's newer list names only Shakespeare");
        assert_eq!(ranked[0].handler().id, shakespeare.id);
    }

    #[test]
    fn a_viewer_follows_their_kind_3_and_git_authors() {
        let (viewer, a, b) = (Keys::generate(), Keys::generate(), Keys::generate());
        let lists = vec![
            signed(&viewer, 3, 1, &[&["p", &b.public_key().to_hex()]]),
            signed(&viewer, 3, 2, &[&["p", &a.public_key().to_hex()]]),
            signed(&viewer, 10_017, 1, &[&["p", &b.public_key().to_hex()]]),
        ];
        let follows = followed_by(&viewer.public_key(), &lists);
        assert!(follows.contains(&viewer.public_key()));
        assert!(follows.contains(&a.public_key()));
        assert!(
            follows.contains(&b.public_key()),
            "from the git authors list"
        );
        assert_eq!(follows.len(), 3);
    }

    #[test]
    fn remix_is_offered_only_by_a_builder_known_to_clone_nostr_repos() {
        let builder = Keys::generate();
        let shakespeare = handler(&builder, "sk", &["web", "https://shakespeare.diy/<bech32>"]);
        let workshop = handler(
            &builder,
            "gw",
            &["web", "https://gitworkshop.test/<bech32>"],
        );
        let clone = "nostr://npub1x/relay.test/tidepool";
        assert_eq!(
            remix_url(&shakespeare, clone).as_deref(),
            Some("https://shakespeare.diy/clone?url=nostr%3A%2F%2Fnpub1x%2Frelay.test%2Ftidepool")
        );
        assert_eq!(remix_url(&workshop, clone), None);
    }

    #[test]
    fn the_naddr_carries_this_relay_as_its_hint() {
        let owner = Keys::generate().public_key();
        let repo = RepoAddress::new(owner, "tidepool").unwrap();
        let with_hint = naddr(&repo, Some("relay.test"));
        assert_eq!(RepoAddress::from_naddr(&with_hint), Some(repo.clone()));
        assert_ne!(with_hint, naddr(&repo, None));
    }
}
