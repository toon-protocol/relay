//! The pages of one Provider and one Listing, and the `naddr` redirect.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Redirect, Response};
use nostr::event::{Event, Kind};
use nostr::key::PublicKey;
use nostr::nips::nip19::{FromBech32, Nip19, Nip19Coordinate};
use relay_ui::chrome::{self, App, Identicon, Profiles, escape};
use relay_ui::meta::Preview;
use ui_preview::{Card, Face};

use crate::deployment;
use crate::directory::{self, LISTING_KIND, LIVENESS_KIND, Listing, PROFILE_KIND, Provider};
use crate::view;
use crate::{PREFIX, Pages, card_stem, directory_filter, fallback, html_with};

/// A key's name, linking to its `/compute` Provider page.
pub(crate) fn provider_link(names: &HashMap<PublicKey, String>, key: &PublicKey) -> String {
    format!(
        "<a class=\"person-link\" href=\"{}\">{}</a>",
        provider_path(key),
        chrome::name(names, key)
    )
}

pub(crate) fn provider_path(key: &PublicKey) -> String {
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
    let shown = chrome::shown_name(&names, &key);
    let served = deployment::serving(&pages, &key).await;
    let listings = match held.listings.is_empty() {
        true => view::blankslate("This Provider publishes no Listings here."),
        false => view::listing_grid(&pages.apps, &names, &held.listings, false),
    };
    let body = format!(
        "<nav class=\"compute-crumbs\"><a href=\"{PREFIX}\">Compute</a><span>/</span><span>Provider</span></nav>\
         <header class=\"compute-hero\">{avatar}<div class=\"who\"><h1>{name}{hidden}</h1>\
         <div class=\"meta\"><span class=\"npub\" title=\"{npub}\">{short}</span>{social}</div></div>\
         <div class=\"side\">{status}</div></header>\
         <div class=\"split\"><div class=\"main\">\
         <section class=\"compute-section\"><div class=\"section-head\"><h2 class=\"section-title\">Listings \
         <span class=\"count\">{count}</span></h2></div>{listings}</section>{serving}{activity}</div>\
         <aside class=\"compute-side\">{about}</aside></div>",
        avatar = chrome::person_avatar(&pages.apps, &key, false),
        name = escape(&shown),
        hidden = if held.provider.hidden {
            view::ANYONE_ONLY
        } else {
            ""
        },
        npub = chrome::npub(&key),
        short = short_npub(&key),
        social = match pages.apps.serves(App::Social) {
            true => format!(
                "<a data-social href=\"{}\">Social profile</a>",
                chrome::profile_path(&key)
            ),
            false => String::new(),
        },
        status = view::provider_status(held.live_since),
        count = held.listings.len(),
        serving = deployment::serving_section(&pages, &served).await,
        activity = crate::activity::section(&pages, &key, &served).await,
        about = provider_about(&held.provider),
    );
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
    html_with(&pages, StatusCode::OK, &named, &body, &preview)
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
                resources: view::spec_text(listing),
                per_hour: listing
                    .per_hour()
                    .map(|hour| format!("{} per hour", view::hourly(hour))),
                live: listing.live,
            }),
        ))
    }
    .await;
    pages.previews.respond(&headers, card).await
}

/// `npub1abcdefgh…uvwxyz`, the whole of it in its title.
pub(crate) fn short_npub(key: &PublicKey) -> String {
    let npub = chrome::npub(key);
    format!("{}…{}", &npub[..12], &npub[npub.len() - 6..])
}

/// A long token or address as its two ends, the whole of it in its title.
fn short_code(text: &str) -> String {
    let shown = match text.chars().count() > 18 {
        true => {
            let chars: Vec<char> = text.chars().collect();
            format!(
                "{}…{}",
                chars[..8].iter().collect::<String>(),
                chars[chars.len() - 6..].iter().collect::<String>()
            )
        }
        false => text.to_owned(),
    };
    format!("<code title=\"{}\">{}</code>", escape(text), escape(&shown))
}

/// The Provider's Profile fields, for the side of its page.
fn provider_about(provider: &Provider) -> String {
    let mut rows: Vec<(&str, String)> = Vec::new();
    if let Some(isolation) = &provider.isolation {
        rows.push(("Isolation", escape(isolation)));
    }
    match (&provider.host, provider.hidden) {
        (_, true) => rows.push((
            "Host",
            "None published: reachable only through Anyone".to_owned(),
        )),
        (Some(host), false) => rows.push(("Host", format!("<code>{}</code>", escape(host)))),
        (None, false) => {}
    }
    if let Some(seconds) = provider.liveness_interval_s {
        rows.push(("Liveness", format!("every {}", view::interval(seconds))));
    }
    let mut out = String::from("<div class=\"side-card\"><h2>About this Provider</h2>");
    if !rows.is_empty() {
        out.push_str("<dl class=\"kv\">");
        for (label, value) in rows {
            out.push_str(&format!("<dt>{label}</dt><dd>{value}</dd>"));
        }
        out.push_str("</dl>");
    }
    if !provider.settlement.is_empty() {
        out.push_str("<h3>Settlement</h3><ul data-settlement>");
        for (chain, tokens) in &provider.settlement {
            let tokens: String = tokens.iter().map(|t| short_code(t)).collect();
            out.push_str(&format!(
                "<li><span class=\"chip\">{}</span>{tokens}</li>",
                escape(chain)
            ));
        }
        out.push_str("</ul>");
    }
    if !provider.relay_set.is_empty() {
        out.push_str("<h3>Relay Set</h3><ul>");
        for relay in &provider.relay_set {
            out.push_str(&format!("<li><code>{}</code></li>", escape(relay)));
        }
        out.push_str("</ul>");
    }
    out.push_str("</div>");
    out
}

/// Rows of label and value, as a definition list.
pub(crate) fn facts(rows: &[(&str, String)]) -> String {
    let mut out = String::from("<dl class=\"kv\">");
    for (label, value) in rows {
        out.push_str(&format!("<dt>{label}</dt><dd>{value}</dd>"));
    }
    out.push_str("</dl>");
    out
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
    let mut details: Vec<(&str, String)> = Vec::new();
    if let Some(arch) = &listing.arch {
        details.push(("Arch", escape(arch)));
    }
    if let Some(isolation) = &listing.isolation {
        details.push(("Isolation", escape(isolation)));
    }
    details.push((
        "Capabilities",
        match listing.capabilities.is_empty() {
            true => "None stated".to_owned(),
            false => listing
                .capabilities
                .iter()
                .map(|c| format!("<span class=\"chip chip-topic\">{}</span>", escape(c)))
                .collect::<Vec<_>>()
                .join(" "),
        },
    ));
    if let Some(s) = listing.lease_interval_s {
        details.push(("Lease Interval", view::interval(s)));
    }
    let spec = view::spec_html(listing, true);
    let body = format!(
        "<nav class=\"compute-crumbs\"><a href=\"{PREFIX}\">Compute</a><span>/</span>\
         <a href=\"{provider}\">{provider_name}</a><span>/</span><span>Listing</span></nav>\
         <header class=\"compute-hero\"><div class=\"who\"><h1>{name}{chip}</h1>\
         <div class=\"meta\">{avatar}<a href=\"{provider}\">{provider_name}</a>{hidden}</div></div></header>\
         <div class=\"split\"><div class=\"main\">\
         <div class=\"price-panel\">{price}<span class=\"muted small\">{standby}</span></div>\
         {resources}\
         <section class=\"compute-section\"><h2 class=\"section-title\">Details</h2>\
         <div class=\"box box-body compute-side\">{details}</div></section></div>\
         <aside class=\"compute-side\">{about}</aside></div>",
        provider = provider_path(&key),
        provider_name = escape(&chrome::shown_name(&names, &key)),
        name = escape(&listing.name),
        chip = view::status_chip(listing),
        avatar = chrome::person_avatar(&pages.apps, &key, true),
        hidden = if listing.hidden {
            view::ANYONE_ONLY
        } else {
            ""
        },
        price = view::price_tag(listing),
        standby = view::standby_text(listing),
        resources = match spec.is_empty() {
            true => String::new(),
            false => format!(
                "<section class=\"compute-section\"><h2 class=\"section-title\">Resources</h2>{spec}</section>"
            ),
        },
        details = facts(&details),
        about = provider_about(&held.provider),
    );
    let description = match (listing.per_hour(), view::spec_text(listing)) {
        (Some(hour), resources) if !resources.is_empty() => {
            format!("{resources}, {} per hour.", view::hourly(hour))
        }
        (Some(hour), _) => format!("{} per hour.", view::hourly(hour)),
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
