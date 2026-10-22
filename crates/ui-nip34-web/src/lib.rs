//! The browser half of the NIP-34 (git) app UI (#283).
//!
//! A placeholder: it writes "hello" into the page's `#app` element, to prove
//! the pipeline from `cargo xtask ui-assets` to a hashed `.wasm` the relay
//! serves. The pages that browse repositories come later.

use wasm_bindgen::prelude::wasm_bindgen;

/// What the placeholder writes into the page.
pub const GREETING: &str = "hello";

/// Runs when the module is instantiated.
#[wasm_bindgen(start)]
pub fn start() {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    if let Some(app) = document.get_element_by_id("app") {
        app.set_text_content(Some(GREETING));
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_greeting_is_hello() {
        assert_eq!(super::GREETING, "hello");
    }
}
