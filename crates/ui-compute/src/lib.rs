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
//! A provider's page also carries its activity log ([`activity`]).
//!
//! The words are TOON Compute's: Provider, Listing, Liveness.

mod activity;
mod deployment;
mod directory;
mod provider;
mod view;

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
use provider::{by_naddr, listing_page, provider_page, unnamed_listing_page};

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

/// What `/repo`'s Environments tab draws with: the Deployments of a
/// repository that its owner and maintainers signed (see [`deployment`]).
pub fn environments(store: ReadOnlyStore, apps: Apps) -> ui_nip34::Environments {
    std::sync::Arc::new(move |repo, authorized| {
        let (store, apps) = (store.clone(), apps.clone());
        Box::pin(async move { deployment::environments(&store, &apps, &repo, &authorized).await })
    })
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
    let options = FilterOptions::of(&listings);
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
        &directory_page(&pages.apps, &names, &listings, &filters, &options),
    )
}

fn directory_page(
    apps: &Apps,
    names: &HashMap<PublicKey, String>,
    listings: &[Listing],
    filters: &Filters,
    options: &FilterOptions,
) -> String {
    let providers: std::collections::HashSet<&PublicKey> =
        listings.iter().map(|l| &l.provider).collect();
    let live: std::collections::HashSet<&PublicKey> = listings
        .iter()
        .filter(|l| l.live)
        .map(|l| &l.provider)
        .collect();
    let stat = |n: usize, one: &str, many: &str| {
        format!(
            "<div><strong>{n}</strong>{}</div>",
            if n == 1 { one } else { many }
        )
    };
    let mut body = format!(
        "<div class=\"stack\"><div class=\"compute-head\"><div><h1 class=\"page-title\">Compute</h1>\
         <p class=\"lead\">Rent a slice of a Provider's machine for a Lease Interval at a time, \
         paid in µUSDC. Live Providers come first, then the cheapest per hour.</p>\
         <p class=\"compute-note\" data-notice=\"relay-only\">This lists only Providers that publish \
         to this relay. A Provider whose Relay Set leaves this relay out is not listed here.</p></div>\
         <div class=\"compute-stats\">{}{}{}</div></div>",
        stat(providers.len(), "Provider", "Providers"),
        stat(live.len(), "live", "live"),
        stat(listings.len(), "Listing", "Listings"),
    );
    body.push_str(&filter_form(filters, options));
    if listings.is_empty() {
        body.push_str(&view::blankslate(match filters.is_empty() {
            true => "No Listings here yet.",
            false => "No Listings match these filters.",
        }));
    } else {
        // Live first already; the down ones get their own heading.
        let down = listings.partition_point(|l| l.live);
        let (live, down) = listings.split_at(down);
        if !live.is_empty() {
            body.push_str(&view::listing_grid(apps, names, live, true));
        }
        if !down.is_empty() {
            body.push_str(&format!(
                "<h2 class=\"group-head\" data-group=\"down\">Not live right now · {}</h2>",
                down.len()
            ));
            body.push_str(&view::listing_grid(apps, names, down, true));
        }
    }
    body.push_str("</div>");
    body
}

/// A card's file name `<stem>.png`: the stem, which may be empty (a Listing
/// or a Deployment whose `d` is empty).
pub(crate) fn card_stem(file: &str) -> Option<&str> {
    file.strip_suffix(".png")
}

fn now() -> u64 {
    u64::try_from(chrome::now()).unwrap_or(0)
}

/// The values the directory's filters can take: those its Listings hold.
pub(crate) struct FilterOptions {
    arch: Vec<String>,
    isolation: Vec<String>,
    gpu: Vec<String>,
    cap: Vec<String>,
}

impl FilterOptions {
    fn of(listings: &[Listing]) -> Self {
        let collect = |values: &mut dyn Iterator<Item = String>| {
            let mut values: Vec<String> = values.map(|v| v.to_lowercase()).collect();
            values.sort();
            values.dedup();
            values
        };
        Self {
            arch: collect(&mut listings.iter().filter_map(|l| l.arch.clone())),
            isolation: collect(&mut listings.iter().filter_map(|l| l.isolation.clone())),
            gpu: collect(&mut listings.iter().filter_map(|l| l.gpu().map(str::to_owned))),
            cap: collect(&mut listings.iter().flat_map(|l| l.capabilities.clone())),
        }
    }
}

/// The GET form that narrows the directory: it works without script, keeps
/// the current values, and its URL is the filtered page.
fn filter_form(filters: &Filters, options: &FilterOptions) -> String {
    let select = |name: &str, label: &str, current: &str, values: &[String]| {
        let mut values = values.to_vec();
        if !current.is_empty() && !values.iter().any(|v| v.eq_ignore_ascii_case(current)) {
            values.push(current.to_owned());
        }
        let options: String = values
            .iter()
            .map(|value| {
                format!(
                    "<option value=\"{0}\"{1}>{0}</option>",
                    escape(value),
                    if value.eq_ignore_ascii_case(current) {
                        " selected"
                    } else {
                        ""
                    }
                )
            })
            .collect();
        format!(
            "<label>{label}<select name=\"{name}\"><option value=\"\">Any</option>{options}</select></label>"
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
    let clear = match filters.is_empty() {
        true => String::new(),
        false => format!("<a href=\"{PREFIX}\">Clear</a>"),
    };
    format!(
        "<form method=\"get\" action=\"{PREFIX}\" class=\"filter-bar\" data-filters>{}{}{}{}\
         <label>Reachability<select name=\"hidden\">{}{}{}</select></label>\
         <div class=\"go\"><button class=\"btn\" type=\"submit\">Filter</button>{clear}</div></form>",
        select("arch", "Arch", &filters.arch, &options.arch),
        select(
            "isolation",
            "Isolation",
            &filters.isolation,
            &options.isolation
        ),
        select("gpu", "GPU", &filters.gpu, &options.gpu),
        select("cap", "Capability", &filters.cap, &options.cap),
        hidden(None, "Any"),
        hidden(Some(false), "Public host"),
        hidden(Some(true), "Anyone"),
    )
}
