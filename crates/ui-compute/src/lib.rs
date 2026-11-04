//! The compute app UI, mounted at `/compute`: TOON Compute's Provider
//! Directory as this relay's store holds it (ADR 0012, `docs/apps/compute.md`).
//!
//! Server-rendered pages with no script, read through a [`ReadOnlyStore`]
//! that never writes and fetches nothing (ADR 0001):
//!
//! - `/compute`: every Listing whose Provider Profile is in the store, with
//!   its provider ([`directory`]).
//!
//! The words are TOON Compute's: Provider, Listing, Liveness.

mod directory;

use std::collections::HashMap;

use axum::Router;
use axum::extract::{Path, State};
use axum::http::header::{CONTENT_SECURITY_POLICY, REFERRER_POLICY};
use axum::http::{HeaderMap, HeaderValue, StatusCode};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use nostr::event::Kind;
use nostr::filter::Filter;
use nostr::key::PublicKey;
use relay_ui::chrome::{self, App, Apps, Profiles, Section, escape};
use relay_ui::meta::Preview;
use relay_ui::{AppUi, ReadOnlyStore};
use ui_preview::{Mark, Previews};

use directory::{LISTING_KIND, Listing, PROFILE_KIND};

/// The mount prefix.
pub const PREFIX: &str = "/compute";

/// What a `/compute` page may load: its own stylesheet and nothing else. No
/// script, and a form may only submit back to this origin.
pub const CONTENT_SECURITY_POLICY_VALUE: &str = "default-src 'self'; script-src 'none'; \
     img-src 'self'; form-action 'self'; base-uri 'none'";

/// The most Profiles and Listings one page reads.
const READ_LIMIT: usize = 1_000;

/// What the pages read.
#[derive(Clone)]
struct Pages {
    store: ReadOnlyStore,
    apps: Apps,
    /// Link preview cards drawn so far.
    previews: Previews,
}

/// The app UI over `store`, with the `apps` the relay serves (for the site
/// bar and the links to `/social`).
pub fn app(store: ReadOnlyStore, apps: Apps) -> AppUi {
    let router = Router::new()
        .route(PREFIX, get(index))
        .route("/compute/assets/{name}", get(asset))
        .route("/compute/og/site.png", get(site_card))
        .fallback(fallback)
        .with_state(Pages {
            store,
            apps,
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
    let host = ui_preview::footer_host(&pages.apps, None);
    let card = ui_preview::site(Mark::Compute, host);
    pages.previews.respond(&headers, Some(card)).await
}

/// A page with `/compute`'s policy.
fn html(pages: &Pages, status: StatusCode, title: &str, body: &str) -> Response {
    let mut response = (
        status,
        Html(chrome::page_with(
            &pages.apps,
            App::Compute,
            Section::Compute,
            title,
            "",
            body,
            &Preview::default(),
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

async fn fallback(State(pages): State<Pages>) -> Response {
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

/// `/compute`: the Provider Directory.
async fn index(State(pages): State<Pages>) -> Response {
    let (profiles, listings) = tokio::join!(
        pages.store.query(
            Filter::new()
                .kind(Kind::from(PROFILE_KIND))
                .limit(READ_LIMIT)
        ),
        pages.store.query(
            Filter::new()
                .kind(Kind::from(LISTING_KIND))
                .limit(READ_LIMIT)
        ),
    );
    let mut listings = directory::join(&profiles, &listings);
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
    listings.sort_by_cached_key(|l| {
        (
            chrome::shown_name(&names, &l.provider).to_lowercase(),
            l.provider,
            l.name.to_lowercase(),
        )
    });
    html(
        &pages,
        StatusCode::OK,
        "Compute",
        &directory_page(&pages.apps, &names, &listings),
    )
}

fn directory_page(apps: &Apps, names: &HashMap<PublicKey, String>, listings: &[Listing]) -> String {
    let mut body = String::from(
        "<h1 class=\"page-title\">Compute</h1>\
         <p class=\"muted small\" data-notice=\"relay-only\">This lists only Providers that publish \
         to this relay. A Provider whose Relay Set leaves this relay out is not listed here.</p>",
    );
    if listings.is_empty() {
        body.push_str("<div class=\"box\"><div class=\"empty\">No Listings here yet.</div></div>");
        return body;
    }
    body.push_str(
        "<div class=\"box\"><div class=\"table-wrap\"><table class=\"data-table\">\
         <thead><tr><th>Provider</th><th>Listing</th><th>Resources</th><th>Arch</th></tr></thead><tbody>",
    );
    for listing in listings {
        let resources = listing
            .resources
            .iter()
            .map(|(what, amount)| match what.is_empty() {
                true => escape(amount),
                false => format!("{} {}", escape(what), escape(amount)),
            })
            .collect::<Vec<_>>()
            .join(", ");
        body.push_str(&format!(
            "<tr data-listing=\"{}\"><td><span>{}{}</span></td><td class=\"strong\">{}</td>\
             <td>{}</td><td>{}</td></tr>",
            escape(&listing.name),
            chrome::person_avatar(apps, &listing.provider, true),
            chrome::person(apps, names, &listing.provider),
            escape(&listing.name),
            resources,
            listing.arch.as_deref().map(escape).unwrap_or_default(),
        ));
    }
    body.push_str("</tbody></table></div></div>");
    body
}
