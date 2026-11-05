//! The pages of one Provider and one Listing, and the `naddr` redirect.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use nostr::event::{Event, Kind};
use nostr::key::PublicKey;
use nostr::nips::nip19::{FromBech32, Nip19, Nip19Coordinate};
use nostr::types::Timestamp;
use relay_ui::chrome::{self, App, Apps, Identicon, Profiles, escape};
use relay_ui::meta::Preview;
use ui_preview::{Card, Face};

use crate::deployment;
use crate::directory::{self, LISTING_KIND, LIVENESS_KIND, Listing, PROFILE_KIND, Provider};
use crate::{
    PREFIX, Pages, capability_list, card_stem, directory_filter, fallback, html_with, per_hour,
    price, resources, resources_text, standby, status,
};

/// A key's name, linking to its `/compute` Provider page.
pub(crate) fn provider_link(names: &HashMap<PublicKey, String>, key: &PublicKey) -> String {
    format!(
        "<a class=\"person-link\" href=\"{}\">{}</a>",
        provider_path(key),
        chrome::name(names, key)
    )
}

fn provider_path(key: &PublicKey) -> String {
    format!("{PREFIX}/p/{}", chrome::npub(key))
}

/// A Listing's page: `/compute/<npub>/<d>`.
pub(crate) fn listing_path(listing: &Listing) -> String {
    format!(
        "{PREFIX}/{}/{}",
        chrome::npub(&listing.provider),
        chrome::percent_encode(&listing.d)
    )
}

/// The key a path names: an `npub`, an `nprofile` or 64 hex.
pub(crate) fn parse_pubkey(text: &str) -> Option<PublicKey> {
    let text = text.trim();
    let text = text.strip_prefix("nostr:").unwrap_or(text);
    if text.len() == 64 {
        return PublicKey::from_hex(text).ok();
    }
    match Nip19::from_bech32(text).ok()? {
        Nip19::Pubkey(key) => Some(key),
        Nip19::Profile(profile) => Some(profile.public_key),
        _ => None,
    }
}

/// What `key` signed that a page of theirs reads: Profile, Listings, Liveness.
struct Held {
    provider: Provider,
    listings: Vec<Listing>,
    /// When its current Liveness was signed, while it is live.
    live_since: Option<u64>,
}

/// `key`'s Provider and its Listings, as the store holds them; `None` when
/// it holds no Provider Profile for it.
async fn held(pages: &Pages, key: PublicKey) -> Option<Held> {
    let of = |kind: u16| directory_filter(kind).author(key);
    let (profiles, listings, liveness): (Vec<Event>, Vec<Event>, Vec<Event>) = tokio::join!(
        pages.store.query(of(PROFILE_KIND)),
        pages.store.query(of(LISTING_KIND)),
        pages.store.query(of(LIVENESS_KIND)),
    );
    let mut provider = directory::provider(&profiles, &key)?;
    let now = crate::now();
    let mut listings = directory::join(&profiles, &listings);
    // A Listing marked hidden makes its Provider Anyone-only, as the
    // directory shows it: no page names its host.
    if listings.iter().any(|listing| listing.hidden) {
        provider.hidden = true;
        provider.host = None;
    }
    directory::apply_liveness(&mut listings, &liveness, now);
    listings.sort_by_cached_key(|l| (l.name.to_lowercase(), l.d.clone()));
    let live_since = directory::live_since(&liveness, &key, now);
    Some(Held {
        provider,
        listings,
        live_since,
    })
}

/// The name a Provider is shown under: its Profile's own, else its kind 0's.
async fn names(pages: &Pages, provider: &Provider) -> HashMap<PublicKey, String> {
    let mut names = Profiles::load(&pages.store, &[provider.key])
        .await
        .into_names();
    if let Some(name) = &provider.name {
        names.insert(provider.key, name.clone());
    }
    names
}

/// `/compute/p/<npub>`: one Provider.
pub(crate) async fn provider_page(
    State(pages): State<Pages>,
    Path(npub): Path<String>,
) -> Response {
    let Some(held) = async {
        let key = parse_pubkey(&npub)?;
        held(&pages, key).await
    }
    .await
    else {
        return fallback(State(pages)).await;
    };
    let key = held.provider.key;
    let names = names(&pages, &held.provider).await;
    let shown = chrome::plain_name(&names, &key);
    let mut body = format!(
        "<p class=\"muted small\"><a href=\"{PREFIX}\">Compute</a></p>\
         <h1 class=\"page-title\">{}{}</h1>",
        chrome::person_avatar(&pages.apps, &key, false),
        escape(&shown),
    );
    body.push_str(&provider_facts(
        &pages.apps,
        &held.provider,
        held.live_since,
    ));
    body.push_str("<h2>Listings</h2>");
    if held.listings.is_empty() {
        body.push_str("<div class=\"box\"><div class=\"empty\">No Listings here.</div></div>");
    } else {
        body.push_str(&listing_table(&held.listings));
    }
    let served = deployment::serving(&pages, &key).await;
    body.push_str(&deployment::serving_section(&pages, &served).await);
    body.push_str(&crate::activity::section(&pages, &key, &served).await);
    // A key with no name is its npub's start, as the cards name it, not
    // the initials the page heads with.
    let named = chrome::shown_name(&names, &key);
    let preview = Preview::new(
        named.clone(),
        format!(
            "{named}: a TOON Compute Provider, {}, with {}.",
            liveness_word(held.live_since.is_some()),
            count(held.listings.len(), "Listing"),
        ),
    )
    .at(provider_path(&key))
    .image(
        provider_card_path(&key),
        format!("{named}, a Compute Provider"),
    );
    html_with(&pages, StatusCode::OK, &shown, &body, &preview)
}

fn liveness_word(live: bool) -> &'static str {
    match live {
        true => "live",
        false => "down",
    }
}

fn count(number: usize, noun: &str) -> String {
    match number {
        1 => format!("1 {noun}"),
        n => format!("{n} {noun}s"),
    }
}

/// A Provider's card.
fn provider_card_path(key: &PublicKey) -> String {
    format!("{PREFIX}/og/p/{}.png", chrome::npub(key))
}

/// A Listing's card.
fn listing_card_path(listing: &Listing) -> String {
    format!(
        "{PREFIX}/og/l/{}/{}.png",
        chrome::npub(&listing.provider),
        chrome::percent_encode(&listing.d)
    )
}

/// `/compute/og/p/<npub>.png`.
pub(crate) async fn provider_card(
    State(pages): State<Pages>,
    Path(file): Path<String>,
    headers: HeaderMap,
) -> Response {
    let card = async {
        let key = parse_pubkey(ui_preview::stem(&file)?)?;
        let held = held(&pages, key).await?;
        let names = names(&pages, &held.provider).await;
        Some(Card::new(
            pages.card_host(),
            Face::Provider(ui_preview::Provider {
                name: chrome::shown_name(&names, &key),
                avatar: Identicon::of(&key),
                live: held.live_since.is_some(),
                listings: held.listings.len(),
            }),
        ))
    }
    .await;
    pages.previews.respond(&headers, card).await
}

/// `/compute/og/l/<npub>/<d>.png`.
pub(crate) async fn listing_card(
    State(pages): State<Pages>,
    Path((npub, file)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let card = async {
        let key = parse_pubkey(&npub)?;
        let d = card_stem(&file)?;
        let held = held(&pages, key).await?;
        let listing = held.listings.iter().find(|l| l.d == d)?;
        let names = names(&pages, &held.provider).await;
        Some(Card::new(
            pages.card_host(),
            Face::Listing(ui_preview::Listing {
                name: listing.name.clone(),
                provider: chrome::shown_name(&names, &key),
                avatar: Identicon::of(&key),
                resources: resources_text(listing),
                per_hour: listing
                    .per_hour()
                    .map(|hour| format!("{} per hour", per_hour(hour))),
                live: listing.live,
            }),
        ))
    }
    .await;
    pages.previews.respond(&headers, card).await
}

/// The Provider's Profile fields.
fn provider_facts(apps: &Apps, provider: &Provider, live_since: Option<u64>) -> String {
    let mut rows = vec![(
        "Status",
        match live_since {
            Some(at) => format!(
                "<span data-status=\"live\">live</span> \
                 <span class=\"muted small\" data-live-since=\"{at}\">Liveness signed {}</span>",
                chrome::ago(Timestamp::from_secs(at)),
            ),
            None => "<span data-status=\"down\">down</span>".to_owned(),
        },
    )];
    if provider.hidden {
        rows.push((
            "Visibility",
            "<span data-hidden=\"true\">Anyone-only</span>".to_owned(),
        ));
    }
    if let Some(isolation) = &provider.isolation {
        rows.push(("Isolation", escape(isolation)));
    }
    if let Some(host) = &provider.host {
        rows.push(("Host", escape(host)));
    }
    if !provider.settlement.is_empty() {
        let chains = provider
            .settlement
            .iter()
            .map(|(chain, tokens)| match tokens.is_empty() {
                true => escape(chain),
                false => format!(
                    "{}: {}",
                    escape(chain),
                    tokens
                        .iter()
                        .map(|token| escape(token))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            })
            .collect::<Vec<_>>()
            .join("; ");
        rows.push((
            "Settlement",
            format!("<span data-settlement>{chains}</span>"),
        ));
    }
    if !provider.relay_set.is_empty() {
        let relays = provider
            .relay_set
            .iter()
            .map(|relay| format!("<code>{}</code>", escape(relay)))
            .collect::<Vec<_>>()
            .join(" ");
        rows.push(("Relay Set", relays));
    }
    if let Some(seconds) = provider.liveness_interval_s {
        rows.push(("Liveness every", format!("{seconds} s")));
    }
    if apps.serves(App::Social) {
        rows.push((
            "Profile",
            format!(
                "<a data-social href=\"{}\">Social profile</a>",
                chrome::profile_path(&provider.key)
            ),
        ));
    }
    facts(&rows)
}

pub(crate) fn facts(rows: &[(&str, String)]) -> String {
    let mut out = String::from(
        "<div class=\"box\"><div class=\"table-wrap\"><table class=\"data-table\"><tbody>",
    );
    for (label, value) in rows {
        out.push_str(&format!("<tr><th>{label}</th><td>{value}</td></tr>"));
    }
    out.push_str("</tbody></table></div></div>");
    out
}

/// A Provider's Listings, one per row.
fn listing_table(listings: &[Listing]) -> String {
    let mut body = String::from(
        "<div class=\"box\"><div class=\"table-wrap\"><table class=\"data-table\">\
         <thead><tr><th>Listing</th><th>Resources</th><th>Arch</th><th>Status</th><th>Price</th>\
         <th>Standby</th></tr></thead><tbody>",
    );
    for listing in listings {
        body.push_str(&format!(
            "<tr data-listing=\"{}\"><td class=\"strong\"><a href=\"{}\">{}</a></td>\
             <td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape(&listing.name),
            listing_path(listing),
            escape(&listing.name),
            resources(listing),
            listing.arch.as_deref().map(escape).unwrap_or_default(),
            status(listing),
            price(listing),
            standby(listing),
        ));
    }
    body.push_str("</tbody></table></div></div>");
    body
}

/// `/compute/<npub>/<listing>`: one Listing with its Provider.
pub(crate) async fn listing_page(
    State(pages): State<Pages>,
    Path((npub, d)): Path<(String, String)>,
) -> Response {
    let Some(held) = async {
        let key = parse_pubkey(&npub)?;
        held(&pages, key).await
    }
    .await
    else {
        return fallback(State(pages)).await;
    };
    let key = held.provider.key;
    let Some(listing) = held.listings.iter().find(|l| l.d == d) else {
        return fallback(State(pages)).await;
    };
    let names = names(&pages, &held.provider).await;
    let mut rows = vec![
        ("Provider", provider_link(&names, &key)),
        ("Status", status(listing)),
        ("Resources", resources(listing)),
    ];
    if let Some(arch) = &listing.arch {
        rows.push(("Arch", escape(arch)));
    }
    if let Some(isolation) = &listing.isolation {
        rows.push(("Isolation", escape(isolation)));
    }
    rows.push(("Price", price(listing)));
    rows.push(("Standby", standby(listing)));
    if !listing.capabilities.is_empty() {
        rows.push(("Capabilities", capability_list(listing)));
    }
    let body = format!(
        "<p class=\"muted small\"><a href=\"{PREFIX}\">Compute</a> / <a href=\"{}\">{}</a></p>\
         <h1 class=\"page-title\">{}</h1>{}",
        provider_path(&key),
        escape(&chrome::plain_name(&names, &key)),
        escape(&listing.name),
        facts(&rows),
    );
    let description = match (listing.per_hour(), resources_text(listing)) {
        (Some(hour), resources) if !resources.is_empty() => {
            format!("{resources}, {} per hour.", per_hour(hour))
        }
        (Some(hour), _) => format!("{} per hour.", per_hour(hour)),
        (None, resources) => resources,
    };
    let preview = Preview::new(
        format!("{} · {}", listing.name, chrome::shown_name(&names, &key)),
        description,
    )
    .at(listing_path(listing))
    .image(
        listing_card_path(listing),
        format!("{}, a Compute Listing", listing.name),
    );
    html_with(&pages, StatusCode::OK, &listing.name, &body, &preview)
}

/// `/compute/<npub>/`: the Listing whose `d` is empty, which the two-segment
/// route cannot match.
pub(crate) async fn unnamed_listing_page(
    pages: State<Pages>,
    Path(npub): Path<String>,
) -> Response {
    listing_page(pages, Path((npub, String::new()))).await
}

/// `/compute/<naddr>`: a Listing's `naddr` redirects to its page.
pub(crate) async fn by_naddr(State(pages): State<Pages>, Path(target): Path<String>) -> Response {
    let text = target.strip_prefix("nostr:").unwrap_or(&target);
    let coordinate = Nip19Coordinate::from_bech32(text)
        .ok()
        .map(|naddr| naddr.coordinate)
        .filter(|c| [LISTING_KIND, deployment::DEPLOYMENT_KIND].contains(&c.kind.as_u16()));
    match coordinate {
        Some(c) if c.kind == Kind::from(deployment::DEPLOYMENT_KIND) => {
            Redirect::permanent(&deployment::deployment_path(&c.public_key, &c.identifier))
                .into_response()
        }
        Some(c) => Redirect::permanent(&format!(
            "{PREFIX}/{}/{}",
            chrome::npub(&c.public_key),
            chrome::percent_encode(&c.identifier)
        ))
        .into_response(),
        None => fallback(State(pages)).await,
    }
}
