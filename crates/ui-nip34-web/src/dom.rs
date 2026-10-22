//! Drawing a [`Content`] into the page. Text only goes in through
//! `set_text_content`, never as markup: file names and file contents are
//! other people's.

use web_sys::{Document, Element};

use crate::object::NO_LONGER_HELD;
use crate::{Content, Row};

fn element(document: &Document, name: &str, text: Option<&str>) -> Element {
    // A fixed tag name never fails to create.
    let element = document
        .create_element(name)
        .unwrap_or_else(|_| unreachable!("{name} is a valid element name"));
    if let Some(text) = text {
        element.set_text_content(Some(text));
    }
    element
}

fn row(document: &Document, row: &Row) -> Element {
    let item = element(document, "li", None);
    let label = format!("{}{}", row.name, if row.is_dir { "/" } else { "" });
    match &row.href {
        Some(href) => {
            let link = element(document, "a", Some(&label));
            // Setting an attribute on an element cannot fail for a valid name.
            let _ = link.set_attribute("href", href);
            let _ = item.append_child(&link);
        }
        None => item.set_text_content(Some(&format!("{label} (submodule)"))),
    }
    item
}

/// Replaces `root`'s children with `content`.
pub(crate) fn render(document: &Document, root: &Element, content: &Content) {
    root.set_text_content(None);
    let child = match content {
        Content::Tree(rows) => {
            let list = element(document, "ul", None);
            for entry in rows {
                let _ = list.append_child(&row(document, entry));
            }
            list
        }
        Content::Text(text) => element(document, "pre", Some(text)),
        Content::Binary(size) => {
            element(document, "p", Some(&format!("Binary file, {size} bytes.")))
        }
        Content::Unheld => element(document, "p", Some(NO_LONGER_HELD)),
        Content::NoSuchPath => element(document, "p", Some("No such path in this commit.")),
    };
    let _ = root.append_child(&child);
}
