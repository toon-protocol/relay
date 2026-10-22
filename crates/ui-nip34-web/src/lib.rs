//! The browser half of the NIP-34 (git) app UI (#283, #285).
//!
//! A tree or blob page from `ui-nip34` carries a [`Layout`]: the resolved
//! commit and, per object, the candidate URLs to fetch it from. This module
//! fetches each object over `https`, checks its sha1 and renders the tree or
//! file into the page. The relay never fetches an object (ADR 0001); a
//! missing, oversized or mismatched one shows "this object is no longer held"
//! in its place and the rest of the page still renders.
//!
//! Everything but [`browser`] and [`dom`] is plain Rust over a [`Fetch`]
//! trait, so `cargo test` covers it natively.

mod browser;
mod dom;
mod layout;
mod object;
#[cfg(test)]
mod testing;
mod view;

pub use layout::{Layout, PageKind};
pub use object::{Fetch, FetchError, MAX_OBJECT, NO_LONGER_HELD, Unheld, load};
pub use view::{Content, Row, build};

use wasm_bindgen::prelude::wasm_bindgen;

/// Runs when the module is instantiated: renders the page's `#git` element
/// from its `data-git` layout, and does nothing on a page without one.
#[wasm_bindgen(start)]
pub fn start() {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(root) = document.get_element_by_id("git") else {
        return;
    };
    let Some(layout) = root
        .get_attribute("data-git")
        .and_then(|text| serde_json::from_str::<Layout>(&text).ok())
    else {
        root.set_text_content(Some("This page's layout is unreadable."));
        return;
    };
    wasm_bindgen_futures::spawn_local(async move {
        let content = build(&browser::Browser, &layout).await;
        dom::render(&document, &root, &content);
    });
}
