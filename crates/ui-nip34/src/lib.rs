//! The NIP-34 (git) app UI, mounted at `/repo` (#276).
//!
//! A tracer: one placeholder page, to prove the app is mounted on the read
//! port. The pages that read repository events come later.

use axum::Router;
use axum::response::Html;
use axum::routing::get;
use relay_ui::{AppUi, ReadOnlyStore};

/// The mount prefix.
pub const PREFIX: &str = "/repo";

/// The app UI over `store`.
pub fn app(store: ReadOnlyStore) -> AppUi {
    // The placeholder reads nothing yet; the handle is what the pages will be
    // built from.
    let _ = store;
    AppUi::new(PREFIX, Router::new().route(PREFIX, get(index)))
}

async fn index() -> Html<&'static str> {
    Html(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>Repositories</title></head>\
         <body><h1>Repositories</h1><p>NIP-34 repositories on this relay.</p></body></html>",
    )
}
