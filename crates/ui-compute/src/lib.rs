//! The compute app UI, mounted at `/compute`: TOON Compute's Provider
//! Directory as this relay's store holds it (ADR 0012, `docs/apps/compute.md`).
//!
//! Server-rendered pages with no script, read through a [`ReadOnlyStore`]
//! that never writes and fetches nothing (ADR 0001):
//!
//! - `/compute`: every Listing whose Provider Profile is in the store, with
//!   its provider ([`directory`]).
//!
//! - `/compute/p/<npub>`: one Provider, its Listings and a link to its
//!   `/social` profile;
//! - `/compute/<npub>/<listing>`: one Listing, with its Provider; the `naddr`
//!   form, `/compute/<naddr>`, redirects to it ([`provider`]).
//!
//! - `/compute/d/<npub>/<environment>`: one Deployment and its state; its
//!   `naddr` redirects there ([`deployment`]).
//!
//! The words are TOON Compute's: Provider, Listing, Liveness.

mod deployment;
mod directory;
mod provider;

use std::collections::HashMap;

use axum::Router;
use axum::extract::{Path, RawQuery, State};
use axum::http::header::{CONTENT_SECURITY_POLICY, REFERRER_POLICY};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use nostr::event::Kind;
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use relay_ui::chrome::{self, App, Apps, Profiles, Section, escape};
use relay_ui::meta::Preview;
use relay_ui::{AppUi, ReadOnlyStore};
use ui_preview::{Mark, Previews};

use directory::{Filters, LABEL_NAMESPACE, LISTING_KIND, LIVENESS_KIND, Listing, PROFILE_KIND};
use provider::{
    by_naddr, listing_page, listing_path, provider_link, provider_page, unnamed_listing_page,
};

/// The mount prefix.
pub const PREFIX: &str = "/compute";

/// What a `/compute` page may load: its own stylesheet and nothing else. No
/// script, and a form may only submit back to this origin.
pub const CONTENT_SECURITY_POLICY_VALUE: &str = "default-src 'self'; script-src 'none'; \
     img-src 'self'; form-action 'self'; base-uri 'none'";

/// The most Profiles and Listings one page reads.
pub(crate) const READ_LIMIT: usize = 1_000;

/// What the pages read.
#[derive(Clone)]
pub(crate) struct Pages {
    pub(crate) store: ReadOnlyStore,
    pub(crate) apps: Apps,
    /// The host visitors' clients reach this relay at, for the site card.
    relay_host: Option<String>,
    /// Link preview cards drawn so far.
    previews: Previews,
}

impl Pages {
    /// The host a preview card's footer names.
    pub(crate) fn card_host(&self) -> Option<String> {
        ui_preview::footer_host(&self.apps, self.relay_host.as_deref())
    }
}

/// The app UI over `store`, with the host this relay is reached at
/// (`TOON_UI_RELAY_URL`'s, for the site card) and the `apps` the relay serves
/// (for the site bar and the links to `/social`).
pub fn app(store: ReadOnlyStore, relay_host: Option<String>, apps: Apps) -> AppUi {
    let router = Router::new()
        .route(PREFIX, get(index))
        .route("/compute/p/{npub}", get(provider_page))
        .route(
            "/compute/d/{npub}/{environment}",
            get(deployment::deployment_page),
        )
        .route(
            "/compute/d/{npub}/",
            get(deployment::unnamed_deployment_page),
        )
        .route("/compute/{target}", get(by_naddr))
        .route("/compute/{npub}/{listing}", get(listing_page))
        .route("/compute/{npub}/", get(unnamed_listing_page))
        .route("/compute/assets/{name}", get(asset))
        .route("/compute/og/site.png", get(site_card))
        .route("/compute/og/p/{file}", get(provider::provider_card))
        .route("/compute/og/l/{npub}/{file}", get(provider::listing_card))
        .route(
            "/compute/og/d/{npub}/{file}",
            get(deployment::deployment_card),
        )
        .fallback(fallback)
        .with_state(Pages {
            store,
            apps,
            relay_host,
            previews: Previews::new(),
        });
    AppUi::new(PREFIX, router)
}

async fn asset(Path(name): Path<String>) -> Response {
    if name == relay_ui::stylesheet_name() {
        return relay_ui::stylesheet_response();
    }
    StatusCode::NOT_FOUND.into_response()
}

async fn site_card(State(pages): State<Pages>, headers: HeaderMap) -> Response {
    let card = ui_preview::site(Mark::Compute, pages.card_host());
    pages.previews.respond(&headers, Some(card)).await
}

/// A page with `/compute`'s policy.
pub(crate) fn html(pages: &Pages, status: StatusCode, title: &str, body: &str) -> Response {
    html_with(pages, status, title, body, &Preview::default())
}

/// [`html`], telling link unfurlers what `preview` says: the policy is the
/// same, a card is only a meta tag.
pub(crate) fn html_with(
    pages: &Pages,
    status: StatusCode,
    title: &str,
    body: &str,
    preview: &Preview,
) -> Response {
    let mut response = (
        status,
        Html(chrome::page_with(
            &pages.apps,
            App::Compute,
            Section::Compute,
            title,
            "",
            body,
            preview,
        )),
    )
        .into_response();
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY_VALUE),
    );
    headers.insert(REFERRER_POLICY, HeaderValue::from_static("no-referrer"));
    response
}

pub(crate) async fn fallback(State(pages): State<Pages>) -> Response {
    html(
        &pages,
        StatusCode::NOT_FOUND,
        "Not found",
        &format!(
            "<div class=\"box\"><div class=\"empty\"><h2>Not found</h2>\
             <p>Nothing on this relay lives at this address.</p>\
             <p><a href=\"{PREFIX}\">Back to the directory</a></p></div></div>"
        ),
    )
}

/// The directory's events of `kind`: every TOON Compute event carries
/// `["L","toon.network"]`.
pub(crate) fn directory_filter(kind: u16) -> Filter {
    Filter::new()
        .kind(Kind::from(kind))
        .custom_tag(SingleLetterTag::UPPERCASE_L, LABEL_NAMESPACE)
        .limit(READ_LIMIT)
}

/// `/compute`: the Provider Directory.
async fn index(State(pages): State<Pages>, RawQuery(query): RawQuery) -> Response {
    let filters = Filters::from_query(query.as_deref());
    let (profiles, listings, liveness) = tokio::join!(
        pages.store.query(directory_filter(PROFILE_KIND)),
        pages.store.query(directory_filter(LISTING_KIND)),
        pages.store.query(directory_filter(LIVENESS_KIND)),
    );
    let mut listings = directory::join(&profiles, &listings);
    listings.retain(|l| filters.matches(l));
    let keys: Vec<PublicKey> = {
        let mut keys: Vec<PublicKey> = listings.iter().map(|l| l.provider).collect();
        keys.dedup();
        keys
    };
    let mut names = Profiles::load(&pages.store, &keys).await.into_names();
    // A Provider Profile's own name beats the key's kind 0.
    for listing in &listings {
        if let Some(name) = &listing.provider_name {
            names.insert(listing.provider, name.clone());
        }
    }
    // Name order is the tiebreak under the buyer's order.
    listings.sort_by_cached_key(|l| {
        (
            chrome::shown_name(&names, &l.provider).to_lowercase(),
            l.provider,
            l.name.to_lowercase(),
        )
    });
    directory::apply_liveness(&mut listings, &liveness, now());
    directory::sort_for_buyers(&mut listings);
    html(
        &pages,
        StatusCode::OK,
        "Compute",
        &directory_page(&pages.apps, &names, &listings, &filters),
    )
}

fn directory_page(
    apps: &Apps,
    names: &HashMap<PublicKey, String>,
    listings: &[Listing],
    filters: &Filters,
) -> String {
    let mut body = String::from(
        "<h1 class=\"page-title\">Compute</h1>\
         <p class=\"muted small\" data-notice=\"relay-only\">This lists only Providers that publish \
         to this relay. A Provider whose Relay Set leaves this relay out is not listed here.</p>",
    );
    body.push_str(&filter_form(filters));
    if listings.is_empty() {
        let none = match filters.is_empty() {
            true => "No Listings here yet.",
            false => "No Listings match these filters.",
        };
        body.push_str(&format!(
            "<div class=\"box\"><div class=\"empty\">{none}</div></div>"
        ));
        return body;
    }
    body.push_str(
        "<div class=\"box\"><div class=\"table-wrap\"><table class=\"data-table\">\
         <thead><tr><th>Provider</th><th>Listing</th><th>Resources</th><th>Arch</th><th>Status</th><th>Price</th><th>Standby</th></tr></thead><tbody>",
    );
    for listing in listings {
        body.push_str(&format!(
            "<tr data-listing=\"{}\"><td><span>{}{}</span>{}</td><td class=\"strong\"><a href=\"{}\">{}</a></td>\
             <td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            escape(&listing.name),
            chrome::person_avatar(apps, &listing.provider, true),
            provider_link(names, &listing.provider),
            match listing.hidden {
                true => " <span class=\"muted small\" data-hidden=\"true\">Anyone-only</span>",
                false => "",
            },
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

/// Live with its capacity, or down.
pub(crate) fn status(listing: &Listing) -> String {
    match (listing.live, listing.available) {
        (true, Some(n)) => format!("<span data-status=\"live\">live, {n} available</span>"),
        (true, None) => "<span data-status=\"live\">live</span>".to_owned(),
        (false, _) => "<span data-status=\"down\">down</span>".to_owned(),
    }
}

/// The price per Lease Interval and per hour, exactly.
pub(crate) fn price(listing: &Listing) -> String {
    let Some(price) = listing.price else {
        return String::new();
    };
    let mut out = match listing.lease_interval_s {
        Some(s) => format!("{price} µUSDC per {s} s"),
        None => format!("{price} µUSDC"),
    };
    if let Some(hour) = listing.per_hour() {
        out.push_str(&format!(" ({} per hour)", per_hour(hour)));
    }
    out
}

pub(crate) fn per_hour(hour: directory::PerHour) -> String {
    match hour.remainder {
        0 => format!("{} µUSDC", hour.whole),
        r => format!("{} + {}/{} µUSDC", hour.whole, r, hour.interval),
    }
}

pub(crate) fn standby(listing: &Listing) -> String {
    match listing.standby_price {
        Some(price) => format!("{price} µUSDC"),
        None => "no standbys".to_owned(),
    }
}

/// The Listing's resources as `what amount` pairs.
pub(crate) fn resources(listing: &Listing) -> String {
    escape(&resources_text(listing))
}

/// [`resources`] as plain text, for a card.
pub(crate) fn resources_text(listing: &Listing) -> String {
    listing
        .resources
        .iter()
        .map(|(what, amount)| match what.is_empty() {
            true => amount.clone(),
            false => format!("{what} {amount}"),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// A card's file name `<stem>.png`: the stem, which may be empty (a Listing
/// or a Deployment whose `d` is empty).
pub(crate) fn card_stem(file: &str) -> Option<&str> {
    file.strip_suffix(".png")
}

fn now() -> u64 {
    u64::try_from(chrome::now()).unwrap_or(0)
}

/// The GET form that narrows the directory: it works without script, keeps
/// the current values, and its URL is the filtered page.
fn filter_form(filters: &Filters) -> String {
    let field = |name: &str, label: &str, value: &str| {
        format!(
            "<label class=\"small muted\">{label} <input type=\"text\" name=\"{name}\" \
             value=\"{}\"></label> ",
            escape(value)
        )
    };
    let hidden = |value: Option<bool>, text: &str| {
        let selected = match filters.hidden == value {
            true => " selected",
            false => "",
        };
        let value = value.map(|v| v.to_string()).unwrap_or_default();
        format!("<option value=\"{value}\"{selected}>{text}</option>")
    };
    format!(
        "<form method=\"get\" action=\"{PREFIX}\" class=\"box\" data-filters>{}{}{}{}\
         <label class=\"small muted\">Hidden <select name=\"hidden\">{}{}{}</select></label> \
         <button type=\"submit\">Filter</button> <a href=\"{PREFIX}\">Clear</a></form>",
        field("arch", "Arch", &filters.arch),
        field("isolation", "Isolation", &filters.isolation),
        field("gpu", "GPU", &filters.gpu),
        field("cap", "Capability", &filters.cap),
        hidden(None, "any"),
        hidden(Some(true), "Anyone-only"),
        hidden(Some(false), "not hidden"),
    )
}
