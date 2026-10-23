//! Drawing a [`Content`] into the page. Text only goes in through
//! `set_text_content`, never as markup: file names and file contents are
//! other people's.

use web_sys::{Document, Element};

use crate::object::NO_LONGER_HELD;
use crate::{Body, CommitView, Content, Files, HISTORY_ENDS, Log, Row};

fn element(document: &Document, name: &str, text: Option<&str>) -> Element {
    let element = document
        .create_element(name)
        .expect("every caller passes a fixed, valid tag name");
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

fn anchor(document: &Document, href: &str, text: &str) -> Element {
    let link = element(document, "a", Some(text));
    // Setting an attribute on an element cannot fail for a valid name.
    let _ = link.set_attribute("href", href);
    link
}

fn log(document: &Document, log: &Log) -> Element {
    let section = element(document, "div", None);
    let list = element(document, "ul", None);
    for row in &log.rows {
        let item = element(document, "li", None);
        let _ = item.append_child(&anchor(document, &row.href, &row.summary));
        let detail = format!(" {} {} {}", &row.id.to_string()[..7], row.author, row.date);
        let _ = item.append_child(&element(document, "span", Some(&detail)));
        let _ = list.append_child(&item);
    }
    let _ = section.append_child(&list);
    if log.ended {
        let _ = section.append_child(&element(document, "p", Some(HISTORY_ENDS)));
    }
    let nav = element(document, "p", None);
    if let Some(newer) = &log.newer {
        let _ = nav.append_child(&anchor(document, newer, "Newer commits"));
        let _ = nav.append_child(&element(document, "span", Some(" ")));
    }
    if let Some(older) = &log.older {
        let _ = nav.append_child(&anchor(document, older, "Older commits"));
    }
    let _ = section.append_child(&nav);
    section
}

fn commit(document: &Document, view: &CommitView) -> Element {
    let section = element(document, "div", None);
    let add = |child: Element| {
        let _ = section.append_child(&child);
    };
    add(element(document, "pre", Some(&view.message)));
    add(element(
        document,
        "p",
        Some(&format!("{} on {}", view.author, view.date)),
    ));
    let parents = element(document, "p", Some("Parents:"));
    for parent in &view.parents {
        let _ = parents.append_child(&element(document, "span", Some(" ")));
        let _ = parents.append_child(&anchor(document, &parent.href, &parent.id.to_string()[..7]));
    }
    if view.parents.is_empty() {
        let _ = parents.append_child(&element(document, "span", Some(" none (a root commit)")));
    }
    add(parents);
    let tree = element(document, "p", None);
    let _ = tree.append_child(&anchor(document, &view.tree_href, "Browse files"));
    add(tree);
    match &view.files {
        Files::Unheld => add(element(document, "p", Some(NO_LONGER_HELD))),
        Files::TooMany => add(element(
            document,
            "p",
            Some("This commit changes too many files to show here."),
        )),
        Files::Changed(files) => {
            add(element(
                document,
                "p",
                Some(&format!("{} changed files", files.len())),
            ));
            for file in files {
                add(element(
                    document,
                    "h3",
                    Some(&format!("{}: {}", file.status, file.label)),
                ));
                add(match &file.body {
                    Body::Diff(diff) => element(document, "pre", Some(diff)),
                    Body::Binary => element(document, "p", Some("Binary file differs.")),
                    Body::Unheld => element(document, "p", Some(NO_LONGER_HELD)),
                    Body::Note(note) => element(document, "p", Some(note)),
                });
            }
        }
    }
    section
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
        Content::Log(page) => log(document, page),
        Content::Commit(view) => commit(document, view),
        Content::Text(text) => element(document, "pre", Some(text)),
        Content::Binary(size) => {
            element(document, "p", Some(&format!("Binary file, {size} bytes.")))
        }
        Content::Unheld => element(document, "p", Some(NO_LONGER_HELD)),
        Content::NoSuchPath => element(document, "p", Some("No such path in this commit.")),
    };
    let _ = root.append_child(&child);
}
