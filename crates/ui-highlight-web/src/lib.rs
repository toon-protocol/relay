//! Syntax highlighting for the `/repo` blob page (#287).
//!
//! A separate WebAssembly module from `ui-nip34-web`, which the blob page
//! alone loads: a tree or commit page never references it. It waits for
//! `ui-nip34-web` to put the file into the page's `#git` element as a `<pre>`,
//! chooses the language from the file's extension, and swaps the `<pre>`'s text
//! for coloured runs of the same text. An unknown extension, a file too big to
//! be worth it, or this module failing to load leaves the plain text, which is
//! what the page showed all along.
//!
//! The highlighter is `ui-highlight`, plain Rust that `/social` shares; the
//! colours are the shared stylesheet's classes. [`dom`] is the browser half.

mod dom;

use wasm_bindgen::prelude::wasm_bindgen;

/// Runs when the module is instantiated; does nothing on a page that is not a
/// blob page.
#[wasm_bindgen(start)]
pub fn start() {
    dom::watch();
}
