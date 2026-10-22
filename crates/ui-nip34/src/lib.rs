//! The NIP-34 (git) app UI, mounted at `/repo` (#276).
//!
//! A tracer: one placeholder page, to prove the app is mounted on the read
//! port and that the browser half (`ui-nip34-web`, built by `cargo xtask
//! ui-assets`) is served from `/repo/assets/`. The pages that read repository
//! events come later.

use axum::Router;
use axum::extract::{Path, State};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use relay_ui::{AppUi, Assets, ReadOnlyStore};

/// The mount prefix.
pub const PREFIX: &str = "/repo";

/// The manifest's name for the page's script.
const SCRIPT: &str = "ui-nip34-web.js";

/// What the page says when there are no assets to browse with.
pub const UNAVAILABLE: &str = "git browsing unavailable";

/// The app UI over `store`, with the built `assets` if the relay has them.
pub fn app(store: ReadOnlyStore, assets: Option<Assets>) -> AppUi {
    // The placeholder reads nothing yet; the handle is what the pages will be
    // built from.
    let _ = store;
    let router = Router::new()
        .route(PREFIX, get(index))
        .route("/repo/assets/{name}", get(asset))
        .with_state(assets);
    AppUi::new(PREFIX, router)
}

async fn index(State(assets): State<Option<Assets>>) -> Html<String> {
    let body = match assets.and_then(|assets| assets.url(PREFIX, SCRIPT)) {
        Some(script) => format!(
            "<p>NIP-34 repositories on this relay.</p><p id=\"app\"></p>\
             <script type=\"module\" src=\"{script}\"></script>"
        ),
        None => format!("<p>{UNAVAILABLE}</p>"),
    };
    Html(format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>Repositories</title></head>\
         <body><h1>Repositories</h1>{body}</body></html>"
    ))
}

async fn asset(State(assets): State<Option<Assets>>, Path(name): Path<String>) -> Response {
    match assets {
        Some(assets) => assets.respond(&name),
        None => axum::http::StatusCode::NOT_FOUND.into_response(),
    }
}
