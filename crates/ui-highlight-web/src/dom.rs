//! Finding the file on the page and drawing runs into it. Text only goes in
//! through `set_text_content`, never as markup: a file's contents are other
//! people's.

use serde::Deserialize;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::prelude::JsValue;
use web_sys::{Document, Element, HtmlElement, MutationObserver, MutationObserverInit};

use crate::highlight::{Run, extension, highlight};

/// The two fields of `ui-nip34-web`'s layout this module reads.
#[derive(Deserialize)]
struct Layout {
    kind: String,
    path: Vec<String>,
}

/// The extension of the file a blob page shows.
fn blob_extension(root: &Element) -> Option<String> {
    let layout: Layout = serde_json::from_str(&root.get_attribute("data-git")?).ok()?;
    if layout.kind != "blob" {
        return None;
    }
    extension(layout.path.last()?).map(str::to_owned)
}

/// Colours the file once `ui-nip34-web` has drawn it, which is some time
/// after this module loads: it fetches the objects first.
pub(crate) fn watch() {
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let Some(root) = document.get_element_by_id("git") else {
        return;
    };
    let Some(extension) = blob_extension(&root) else {
        return;
    };
    let callback = {
        let (document, root, extension) = (document.clone(), root.clone(), extension.clone());
        Closure::<dyn FnMut(JsValue, MutationObserver)>::new(
            move |_records: JsValue, observer: MutationObserver| {
                if colour(&document, &root, &extension) {
                    observer.disconnect();
                }
            },
        )
    };
    let Ok(observer) = MutationObserver::new(callback.as_ref().unchecked_ref()) else {
        return;
    };
    // The observer holds the callback for the life of the page.
    callback.forget();
    let options = MutationObserverInit::new();
    options.set_child_list(true);
    options.set_subtree(true);
    if observer.observe_with_options(&root, &options).is_err() {
        return;
    }
    // The file may be there already.
    if colour(&document, &root, &extension) {
        observer.disconnect();
    }
}

/// Whether the page now holds a file (highlighted or left plain): the watch
/// is over.
fn colour(document: &Document, root: &Element, extension: &str) -> bool {
    let Ok(Some(pre)) = root.query_selector("pre.blob-code") else {
        return false;
    };
    let Some(text) = pre.text_content() else {
        return true;
    };
    let Some(runs) = highlight(extension, &text) else {
        return true;
    };
    pre.set_text_content(None);
    for run in &runs {
        let _ = pre.append_child(&span(document, run));
    }
    let _ = pre.set_attribute("data-highlighted", extension);
    true
}

/// A run as an element. The colour goes through the CSSOM, not a `style`
/// attribute, which the page's content security policy refuses.
fn span(document: &Document, run: &Run) -> Element {
    let span = document
        .create_element("span")
        .expect("span is a valid tag name");
    span.set_text_content(Some(&run.text));
    if let Some(element) = span.dyn_ref::<HtmlElement>() {
        let style = element.style();
        if let Some(rgb) = run.color {
            let _ = style.set_property("color", &format!("#{rgb:06x}"));
        }
        if run.bold {
            let _ = style.set_property("font-weight", "bold");
        }
        if run.italic {
            let _ = style.set_property("font-style", "italic");
        }
    }
    span
}
