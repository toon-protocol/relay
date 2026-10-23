//! Drawing a [`Content`] into the page, in rig-web's look (the classes are
//! `relay_ui`'s stylesheet). Text only goes in through text nodes, never as
//! markup: file names and file contents are other people's. A Markdown
//! document goes in as the fixed tags `ui_markdown` read it into.

use ui_markdown::Node;
use web_sys::{Document, Element};

use crate::object::NO_LONGER_HELD;
use crate::view::{Document as Doc, Head, Listing, Readme};
use crate::{Body, CommitView, Content, Files, HISTORY_ENDS, Log, LogRow, Row};

/// Octicons (MIT, GitHub), 16px.
mod icon {
    pub(super) const DIRECTORY: &str = "M1.75 1A1.75 1.75 0 0 0 0 2.75v10.5C0 14.216.784 15 1.75 15h12.5A1.75 1.75 0 0 0 16 13.25v-8.5A1.75 1.75 0 0 0 14.25 3H7.5a.25.25 0 0 1-.2-.1l-.9-1.2C6.07 1.26 5.55 1 5 1H1.75z";
    pub(super) const FILE: &str = "M3.75 1.5a.25.25 0 0 0-.25.25v12.5c0 .138.112.25.25.25h8.5a.25.25 0 0 0 .25-.25V6H9.75A1.75 1.75 0 0 1 8 4.25V1.5H3.75zm5.75.56v2.19c0 .138.112.25.25.25h2.19L9.5 2.06zM2 1.75C2 .784 2.784 0 3.75 0h5.086c.464 0 .909.184 1.237.513l3.414 3.414c.329.328.513.773.513 1.237v9.086A1.75 1.75 0 0 1 12.25 16h-8.5A1.75 1.75 0 0 1 2 14.25V1.75z";
    pub(super) const SUBMODULE: &str = "M0 2.75C0 1.784.784 1 1.75 1H5c.55 0 1.07.26 1.4.7l.9 1.2a.25.25 0 0 0 .2.1h6.75c.966 0 1.75.784 1.75 1.75v8.5A1.75 1.75 0 0 1 14.25 15H1.75A1.75 1.75 0 0 1 0 13.25Zm9.42 9.36 2.883-2.677a.25.25 0 0 0 0-.366L9.42 6.39a.249.249 0 0 0-.42.183V8.5H4.75a.75.75 0 0 0 0 1.5H9v1.927c0 .218.26.331.42.183Z";
    pub(super) const BOOK: &str = "M0 1.75A.75.75 0 01.75 1h4.253c1.227 0 2.317.59 3 1.501A3.744 3.744 0 0111.006 1h4.245a.75.75 0 01.75.75v10.5a.75.75 0 01-.75.75h-4.507a2.25 2.25 0 00-1.591.659l-.622.621a.75.75 0 01-1.06 0l-.622-.621A2.25 2.25 0 005.258 13H.75a.75.75 0 01-.75-.75zm7.251 10.324l.004-5.073-.002-2.253A2.25 2.25 0 005.003 2.5H1.5v9h3.757a3.75 3.75 0 011.994.574zM8.755 4.75l-.004 7.322a3.752 3.752 0 011.992-.572H14.5v-9h-3.495a2.25 2.25 0 00-2.25 2.25z";
    pub(super) const CODE: &str = "M4.72 3.22a.75.75 0 011.06 1.06L2.06 8l3.72 3.72a.75.75 0 11-1.06 1.06L.47 8.53a.75.75 0 010-1.06l4.25-4.25zm6.56 0a.75.75 0 10-1.06 1.06L13.94 8l-3.72 3.72a.75.75 0 101.06 1.06l4.25-4.25a.75.75 0 000-1.06L11.28 3.22z";
    pub(super) const HISTORY: &str = "m.427 1.927 1.215 1.215a8.002 8.002 0 1 1-1.6 5.685.75.75 0 1 1 1.493-.154 6.5 6.5 0 1 0 1.18-4.458l1.358 1.358A.25.25 0 0 1 3.896 6H.25A.25.25 0 0 1 0 5.75V2.104a.25.25 0 0 1 .427-.177ZM7.75 4a.75.75 0 0 1 .75.75v2.992l2.028.812a.75.75 0 0 1-.557 1.392l-2.5-1A.751.751 0 0 1 7 8.25v-3.5A.75.75 0 0 1 7.75 4Z";
}

const SVG: &str = "http://www.w3.org/2000/svg";

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn element(document: &Document, name: &str, text: Option<&str>) -> Element {
    let element = document
        .create_element(name)
        .expect("every caller passes a fixed, valid tag name");
    if let Some(text) = text {
        element.set_text_content(Some(text));
    }
    element
}

/// An element with a class.
fn styled(document: &Document, name: &str, class: &str, text: Option<&str>) -> Element {
    let element = element(document, name, text);
    element.set_class_name(class);
    element
}

/// Sets an attribute; that cannot fail for the fixed, valid names used here.
fn set(element: &Element, name: &str, value: &str) {
    let _ = element.set_attribute(name, value);
}

fn add(parent: &Element, child: &Element) {
    let _ = parent.append_child(child);
}

fn text(document: &Document, parent: &Element, text: &str) {
    let _ = parent.append_child(&document.create_text_node(text));
}

fn anchor(document: &Document, href: &str, text: &str) -> Element {
    let link = element(document, "a", Some(text));
    set(&link, "href", href);
    link
}

/// An icon, its path drawn in the text colour.
fn svg(document: &Document, path: &str, class: &str) -> Element {
    let svg = document
        .create_element_ns(Some(SVG), "svg")
        .expect("svg is a valid tag name");
    set(&svg, "class", &format!("icon {class}"));
    set(&svg, "viewBox", "0 0 16 16");
    set(&svg, "aria-hidden", "true");
    let shape = document
        .create_element_ns(Some(SVG), "path")
        .expect("path is a valid tag name");
    set(&shape, "d", path);
    add(&svg, &shape);
    svg
}

/// The name in `Name <email>`.
fn name_of(author: &str) -> &str {
    author.split(" <").next().unwrap_or(author)
}

/// A round avatar holding the first two letters of `name`.
fn avatar(document: &Document, name: &str, small: bool) -> Element {
    let initials: String = name
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    let avatar = styled(
        document,
        "span",
        if small { "avatar avatar-sm" } else { "avatar" },
        Some(&initials),
    );
    set(&avatar, "aria-hidden", "true");
    avatar
}

/// `Oct 21, 2026` from `2026-10-21 …`, or the text as it came.
fn day(date: &str) -> String {
    let mut parts = date.get(..10).unwrap_or(date).split('-');
    let (Some(year), Some(month), Some(day)) = (parts.next(), parts.next(), parts.next()) else {
        return date.to_owned();
    };
    match (month.parse::<usize>(), day.parse::<u32>()) {
        (Ok(month @ 1..=12), Ok(day)) => format!("{} {day}, {year}", MONTHS[month - 1]),
        _ => date.to_owned(),
    }
}

/// `1.2 KB`, or bytes below one.
fn size(bytes: usize) -> String {
    if bytes < 1024 {
        format!("{bytes} Bytes")
    } else {
        #[allow(clippy::cast_precision_loss)]
        let kib = bytes as f64 / 1024.0;
        format!("{kib:.2} KB")
    }
}

/// A box saying `message`, alone in the page's content area.
fn empty(document: &Document, message: &str) -> Element {
    let boxed = styled(document, "div", "box", None);
    add(&boxed, &styled(document, "div", "empty", Some(message)));
    boxed
}

/// The fixed tags of a Markdown document, under `parent`.
fn markdown(document: &Document, parent: &Element, nodes: &[Node]) {
    for node in nodes {
        match node {
            Node::Text(words) => text(document, parent, words),
            Node::Element(item) => {
                let child = element(document, item.tag, None);
                for (name, value) in &item.attrs {
                    set(&child, name, value);
                    if *name == "href" && !value.starts_with('#') {
                        set(&child, "rel", "nofollow noopener");
                    }
                }
                markdown(document, &child, &item.children);
                add(parent, &child);
            }
        }
    }
}

fn row(document: &Document, row: &Row) -> Element {
    let item = styled(document, "li", "file-row", None);
    let (path, class) = match (&row.href, row.is_dir) {
        (None, _) => (icon::SUBMODULE, "dir"),
        (Some(_), true) => (icon::DIRECTORY, "dir"),
        (Some(_), false) => (icon::FILE, ""),
    };
    add(&item, &svg(document, path, class));
    match &row.href {
        Some(href) => add(&item, &anchor(document, href, &row.name)),
        None => {
            let label = element(document, "span", Some(&row.name));
            set(&label, "title", "submodule");
            add(&item, &label);
        }
    }
    item
}

/// The bar above a listing: who made the commit, its summary, its id.
fn head(document: &Document, head: &Head) -> Element {
    let bar = styled(document, "div", "commit-bar", None);
    add(&bar, &avatar(document, &head.author, true));
    let message = styled(document, "span", "message truncate", None);
    add(
        &message,
        &styled(document, "span", "strong", Some(&head.author)),
    );
    text(document, &message, " ");
    add(&message, &anchor(document, &head.href, &head.summary));
    add(&bar, &message);
    let id = anchor(document, &head.href, &head.id.to_string()[..7]);
    id.set_class_name("id");
    add(&bar, &id);
    add(
        &bar,
        &styled(document, "span", "small muted", Some(&day(&head.date))),
    );
    bar
}

fn readme(document: &Document, readme: &Readme) -> Element {
    let boxed = styled(document, "div", "box", None);
    let header = styled(document, "div", "box-header", None);
    add(&header, &svg(document, icon::BOOK, ""));
    add(
        &header,
        &styled(document, "span", "strong", Some(&readme.name)),
    );
    add(&boxed, &header);
    let body = styled(document, "div", "box-body-lg", None);
    match &readme.body {
        Doc::Markdown(nodes) => {
            let rendered = styled(document, "div", "markdown", None);
            markdown(document, &rendered, nodes);
            add(&body, &rendered);
        }
        Doc::Plain(words) => {
            let pre = styled(document, "pre", "markdown", Some(words));
            add(&body, &pre);
        }
        Doc::Unheld => add(&body, &styled(document, "p", "muted", Some(NO_LONGER_HELD))),
    }
    add(&boxed, &body);
    boxed
}

fn listing(document: &Document, listing: &Listing) -> Element {
    let section = styled(document, "div", "stack", None);
    let boxed = styled(document, "div", "box", None);
    add(&boxed, &head(document, &listing.head));
    let rows = styled(document, "ul", "rows", None);
    for entry in &listing.rows {
        add(&rows, &row(document, entry));
    }
    add(&boxed, &rows);
    add(&section, &boxed);
    if let Some(found) = &listing.readme {
        add(&section, &readme(document, found));
    }
    section
}

/// A file: its size above it, then its lines beside their numbers. A
/// Markdown file is drawn first, with a switch to its source that needs no
/// script.
fn file(document: &Document, words: &str, nodes: Option<&[Node]>) -> Element {
    let boxed = styled(document, "div", "box", None);
    if nodes.is_some() {
        for (id, checked) in [("view-preview", true), ("view-code", false)] {
            let radio = styled(document, "input", "view-switch", None);
            set(&radio, "type", "radio");
            set(&radio, "name", "view");
            set(&radio, "id", id);
            if checked {
                set(&radio, "checked", "");
            }
            add(&boxed, &radio);
        }
    }
    let header = styled(document, "div", "box-header", None);
    let meta = styled(document, "div", "file-meta", None);
    let lines = words.lines().count();
    add(
        &meta,
        &element(
            document,
            "span",
            Some(&format!(
                "{lines} line{}",
                if lines == 1 { "" } else { "s" }
            )),
        ),
    );
    add(&meta, &element(document, "span", Some(&size(words.len()))));
    add(&header, &meta);
    if nodes.is_some() {
        let switch = styled(document, "div", "segmented", None);
        for (id, label) in [("view-preview", "Preview"), ("view-code", "Code")] {
            let option = element(document, "label", Some(label));
            set(&option, "for", id);
            add(&switch, &option);
        }
        add(&header, &switch);
    }
    add(&boxed, &header);
    if let Some(nodes) = nodes {
        let preview = styled(document, "div", "box-body-lg panel-preview", None);
        let rendered = styled(document, "div", "markdown", None);
        markdown(document, &rendered, nodes);
        add(&preview, &rendered);
        add(&boxed, &preview);
    }
    let code = styled(
        document,
        "div",
        if nodes.is_some() {
            "code panel-code"
        } else {
            "code"
        },
        None,
    );
    let numbers: Vec<String> = (1..=lines.max(1)).map(|n| n.to_string()).collect();
    let gutter = styled(document, "div", "gutter", Some(&numbers.join("\n")));
    set(&gutter, "aria-hidden", "true");
    add(&code, &gutter);
    // The highlighter (ui-highlight-web) colours this element.
    add(&code, &styled(document, "pre", "blob-code", Some(words)));
    add(&boxed, &code);
    boxed
}

fn binary(document: &Document, bytes: usize) -> Element {
    let boxed = styled(document, "div", "box", None);
    let header = styled(document, "div", "box-header", None);
    let meta = styled(document, "div", "file-meta", None);
    add(&meta, &element(document, "span", Some(&size(bytes))));
    add(&header, &meta);
    add(&boxed, &header);
    add(
        &boxed,
        &styled(document, "div", "binary", Some("Binary file not shown.")),
    );
    boxed
}

fn log_row(document: &Document, row: &LogRow) -> Element {
    let item = styled(document, "li", "commit-row", None);
    let dot = styled(document, "span", "dot", None);
    set(&dot, "aria-hidden", "true");
    add(&item, &dot);
    let name = name_of(&row.author);
    add(&item, &avatar(document, name, false));
    let body = styled(document, "div", "body", None);
    let summary = anchor(document, &row.href, &row.summary);
    summary.set_class_name("summary truncate");
    add(&body, &summary);
    let by = styled(document, "div", "small muted", None);
    add(&by, &styled(document, "span", "strong", Some(name)));
    text(document, &by, &format!(" committed on {}", day(&row.date)));
    add(&body, &by);
    add(&item, &body);
    let actions = styled(document, "div", "actions", None);
    let id = anchor(document, &row.href, &row.id.to_string()[..7]);
    id.set_class_name("sha");
    add(&actions, &id);
    let browse = styled(document, "a", "icon-btn", None);
    set(&browse, "href", &row.tree_href);
    set(
        &browse,
        "title",
        "Browse the repository at this point in history",
    );
    set(
        &browse,
        "aria-label",
        "Browse the repository at this commit",
    );
    add(&browse, &svg(document, icon::CODE, ""));
    add(&actions, &browse);
    add(&item, &actions);
    item
}

fn log(document: &Document, log: &Log) -> Element {
    let section = element(document, "div", None);
    let mut group: Option<(String, Element)> = None;
    let close = |section: &Element, group: Option<(String, Element)>| {
        if let Some((_, rows)) = group {
            let boxed = styled(document, "div", "box", None);
            add(&boxed, &rows);
            if let Some(last) = section.last_element_child() {
                add(&last, &boxed);
            }
        }
    };
    for row in &log.rows {
        let today = day(&row.date);
        if group.as_ref().is_none_or(|(shown, _)| *shown != today) {
            close(&section, group.take());
            let block = styled(document, "div", "day-group", None);
            let heading = styled(document, "h3", "day", None);
            add(&heading, &svg(document, icon::HISTORY, "icon-sm"));
            text(document, &heading, &format!(" Commits on {today}"));
            add(&block, &heading);
            add(&section, &block);
            group = Some((today, styled(document, "ul", "rows timeline", None)));
        }
        if let Some((_, rows)) = &group {
            add(rows, &log_row(document, row));
        }
    }
    close(&section, group);
    if log.ended {
        add(&section, &styled(document, "p", "note", Some(HISTORY_ENDS)));
    }
    if log.newer.is_some() || log.older.is_some() {
        let pager = styled(document, "div", "pager", None);
        for (href, label) in [(&log.newer, "Newer"), (&log.older, "Older")] {
            match href {
                Some(href) => {
                    let link = anchor(document, href, label);
                    link.set_class_name("btn");
                    add(&pager, &link);
                }
                None => {
                    let button = styled(document, "button", "btn", Some(label));
                    set(&button, "disabled", "");
                    add(&pager, &button);
                }
            }
        }
        add(&section, &pager);
    }
    section
}

/// Added and removed lines of a diff.
fn counts(diff: &str) -> (usize, usize) {
    diff.lines().fold((0, 0), |(added, removed), line| {
        if line.starts_with('+') {
            (added + 1, removed)
        } else if line.starts_with('-') {
            (added, removed + 1)
        } else {
            (added, removed)
        }
    })
}

/// A diff, each line coloured by what happened to it.
fn diff(document: &Document, diff: &str) -> Element {
    let pre = styled(document, "pre", "diff", None);
    for line in diff.split_inclusive('\n') {
        let class = if line.starts_with('+') {
            "add"
        } else if line.starts_with('-') {
            "del"
        } else if line.starts_with("@@") {
            "hunk"
        } else {
            ""
        };
        add(&pre, &styled(document, "span", class, Some(line)));
    }
    pre
}

/// GitHub's five squares: the share of added and removed lines.
fn diffstat(document: &Document, added: usize, removed: usize) -> Element {
    let bar = styled(document, "span", "diffstat", None);
    set(&bar, "aria-hidden", "true");
    let total = added + removed;
    let share = |part: usize| (part * 5 + total / 2).checked_div(total).unwrap_or(0);
    let greens = share(added).min(5);
    let reds = share(removed).min(5 - greens);
    for index in 0..5 {
        let class = if index < greens {
            "a"
        } else if index < greens + reds {
            "d"
        } else {
            ""
        };
        add(&bar, &styled(document, "i", class, None));
    }
    bar
}

fn commit(document: &Document, view: &CommitView) -> Element {
    let section = styled(document, "div", "stack", None);
    let head = styled(document, "div", "box box-body commit-head", None);
    let mut message = view.message.trim_end().splitn(2, '\n');
    let title = message.next().unwrap_or_default();
    add(&head, &element(document, "h2", Some(title)));
    if let Some(rest) = message
        .next()
        .map(str::trim)
        .filter(|rest| !rest.is_empty())
    {
        add(&head, &element(document, "pre", Some(rest)));
    }
    let by = styled(document, "div", "by", None);
    let name = name_of(&view.author);
    add(&by, &avatar(document, name, true));
    add(&by, &styled(document, "span", "strong", Some(name)));
    add(
        &by,
        &styled(
            document,
            "span",
            "muted",
            Some(&format!("committed on {}", day(&view.date))),
        ),
    );
    let browse = anchor(document, &view.tree_href, "Browse files");
    browse.set_class_name("btn");
    add(&by, &browse);
    add(&head, &by);
    let ids = styled(document, "div", "ids", None);
    let own = element(document, "span", Some("commit "));
    add(
        &own,
        &styled(document, "span", "mono", Some(&view.id.to_string())),
    );
    add(&ids, &own);
    if view.parents.is_empty() {
        add(
            &ids,
            &element(document, "span", Some("no parent (a root commit)")),
        );
    }
    for (index, parent) in view.parents.iter().enumerate() {
        let label = if view.parents.len() > 1 {
            format!("parent {} ", index + 1)
        } else {
            "parent ".to_owned()
        };
        let item = element(document, "span", Some(&label));
        add(
            &item,
            &anchor(document, &parent.href, &parent.id.to_string()[..7]),
        );
        add(&ids, &item);
    }
    add(&head, &ids);
    add(&section, &head);

    match &view.files {
        Files::Unheld => add(&section, &empty(document, NO_LONGER_HELD)),
        Files::TooMany => add(
            &section,
            &empty(document, "This commit changes too many files to show here."),
        ),
        Files::Changed(files) => {
            let boxed = styled(document, "div", "box", None);
            let (added, removed) = files.iter().fold((0, 0), |(a, r), file| match &file.body {
                Body::Diff(text) => {
                    let (plus, minus) = counts(text);
                    (a + plus, r + minus)
                }
                _ => (a, r),
            });
            let header = styled(document, "div", "box-header small", None);
            let summary = element(document, "span", None);
            text(
                document,
                &summary,
                &format!(
                    "Showing {} changed file{} with ",
                    files.len(),
                    if files.len() == 1 { "" } else { "s" }
                ),
            );
            add(
                &summary,
                &styled(
                    document,
                    "span",
                    "added",
                    Some(&format!(
                        "{added} addition{}",
                        if added == 1 { "" } else { "s" }
                    )),
                ),
            );
            text(document, &summary, " and ");
            add(
                &summary,
                &styled(
                    document,
                    "span",
                    "removed",
                    Some(&format!(
                        "{removed} deletion{}",
                        if removed == 1 { "" } else { "s" }
                    )),
                ),
            );
            add(&header, &summary);
            add(&header, &diffstat(document, added, removed));
            add(&boxed, &header);
            for file in files {
                let details = styled(document, "details", "diff-file", None);
                set(&details, "open", "");
                set(&details, "data-status", file.status);
                let summary = element(document, "summary", None);
                let letter = file.status.get(..1).unwrap_or_default().to_uppercase();
                let status = styled(
                    document,
                    "span",
                    &format!("status {}", file.status),
                    Some(&letter),
                );
                set(&status, "title", file.status);
                add(&summary, &status);
                add(
                    &summary,
                    &styled(document, "span", "file-name", Some(&file.label)),
                );
                if let Body::Diff(text) = &file.body {
                    let (plus, minus) = counts(text);
                    let tally = styled(document, "span", "counts", None);
                    add(
                        &tally,
                        &styled(document, "span", "added", Some(&format!("+{plus}"))),
                    );
                    add(&tally, &element(document, "span", Some(" ")));
                    add(
                        &tally,
                        &styled(document, "span", "removed", Some(&format!("-{minus}"))),
                    );
                    add(&summary, &tally);
                }
                add(&details, &summary);
                add(
                    &details,
                    &match &file.body {
                        Body::Diff(text) => diff(document, text),
                        Body::Binary => styled(document, "p", "note", Some("Binary file differs.")),
                        Body::Unheld => styled(document, "p", "note", Some(NO_LONGER_HELD)),
                        Body::Note(note) => styled(document, "p", "note", Some(note)),
                    },
                );
                add(&boxed, &details);
            }
            add(&section, &boxed);
        }
    }
    section
}

/// Replaces `root`'s children with `content`.
pub(crate) fn render(document: &Document, root: &Element, content: &Content) {
    root.set_text_content(None);
    let child = match content {
        Content::Tree(found) => listing(document, found),
        Content::Log(page) => log(document, page),
        Content::Commit(view) => commit(document, view),
        Content::Text { text, markdown } => file(document, text, markdown.as_deref()),
        Content::Binary(bytes) => binary(document, *bytes),
        Content::Unheld => empty(document, NO_LONGER_HELD),
        Content::NoSuchPath => empty(document, "No such path in this commit."),
    };
    add(root, &child);
}
