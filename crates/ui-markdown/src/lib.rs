//! Markdown for the app UIs: a README in the browser, an issue or a comment
//! on the server.
//!
//! A document becomes a tree of [`Node`]s whose tags come from a fixed set
//! and whose only attribute is a link's `href`, which the caller decides.
//! [`to_html`] writes the tree as escaped HTML for a server-rendered page;
//! the browser half builds the same tree as DOM. Raw HTML in a document is
//! never markup: its tags are dropped and the text between them kept, so a
//! README's centred header still reads as words. An image is not loaded (the
//! pages load nothing from elsewhere); it shows as a link to itself, by its
//! alt text.

use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

/// One node of a document.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Node {
    Element(Element),
    Text(String),
}

/// An element: a tag from the fixed set, its attributes, its children.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Element {
    pub tag: &'static str,
    /// Only `href`, and only as the caller's link policy returned it.
    pub attrs: Vec<(&'static str, String)>,
    pub children: Vec<Node>,
}

impl Element {
    fn new(tag: &'static str) -> Self {
        Self {
            tag,
            attrs: Vec::new(),
            children: Vec::new(),
        }
    }
}

/// The scheme of `url`, lowercased, if it names one before any path.
pub fn scheme(url: &str) -> Option<String> {
    let end = url.find([':', '/', '?', '#'])?;
    (url[end..].starts_with(':') && end > 0).then(|| url[..end].to_ascii_lowercase())
}

/// The link policy for a document with no place of its own: `http(s)` and
/// `mailto` links and in-page anchors are followed; anything else is not a
/// link.
pub fn absolute_only(url: &str) -> Option<String> {
    if url.starts_with('#') {
        return Some(url.to_owned());
    }
    match scheme(url)?.as_str() {
        "http" | "https" | "mailto" => Some(url.to_owned()),
        _ => None,
    }
}

/// One open element while the document is read.
struct Frame {
    element: Element,
    /// A `thead`'s cells sit in a row; a code block's text sits in `code`.
    wrap: Option<&'static str>,
    /// The element is dropped and its children kept in its place.
    transparent: bool,
}

/// What a raw HTML run leaves: its text without its tags. A tag or comment
/// left open runs on into the next run.
struct Stripper {
    in_tag: bool,
    in_comment: bool,
}

impl Stripper {
    fn strip(&mut self, html: &str) -> String {
        let mut out = String::new();
        let mut rest = html;
        while !rest.is_empty() {
            if self.in_comment {
                match rest.find("-->") {
                    Some(end) => {
                        self.in_comment = false;
                        rest = &rest[end + 3..];
                    }
                    None => rest = "",
                }
            } else if self.in_tag {
                match rest.find('>') {
                    Some(end) => {
                        self.in_tag = false;
                        rest = &rest[end + 1..];
                    }
                    None => rest = "",
                }
            } else if let Some(start) = rest.find('<') {
                out.push_str(&rest[..start]);
                if rest[start..].starts_with("<!--") {
                    self.in_comment = true;
                    rest = &rest[start + 4..];
                } else {
                    self.in_tag = true;
                    rest = &rest[start + 1..];
                }
            } else {
                out.push_str(rest);
                rest = "";
            }
        }
        decode_entities(&out)
    }
}

/// The few entities raw HTML text commonly holds.
fn decode_entities(text: &str) -> String {
    text.replace("&nbsp;", "\u{a0}")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

fn heading(level: HeadingLevel) -> &'static str {
    match level {
        HeadingLevel::H1 => "h1",
        HeadingLevel::H2 => "h2",
        HeadingLevel::H3 => "h3",
        HeadingLevel::H4 => "h4",
        HeadingLevel::H5 => "h5",
        HeadingLevel::H6 => "h6",
    }
}

/// Reads `text` as GitHub-flavoured Markdown. `link` decides where each link
/// goes: `None` leaves its text unlinked.
pub fn parse(text: &str, link: impl Fn(&str) -> Option<String>) -> Vec<Node> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM;
    let mut stack: Vec<Frame> = vec![Frame {
        element: Element::new("div"),
        wrap: None,
        transparent: true,
    }];
    let mut stripper = Stripper {
        in_tag: false,
        in_comment: false,
    };
    let mut in_head = false;
    // Links open, so an image inside one (a badge) is not a second link.
    let mut links = 0usize;
    let push = |stack: &mut Vec<Frame>, node: Node| {
        if let Some(frame) = stack.last_mut() {
            if let (Node::Text(text), Some(Node::Text(last))) =
                (&node, frame.element.children.last_mut())
            {
                last.push_str(text);
                return;
            }
            frame.element.children.push(node);
        }
    };
    for event in Parser::new_ext(text, options) {
        match event {
            Event::Start(tag) => {
                let (tag, wrap, transparent, href) = match tag {
                    Tag::Paragraph => ("p", None, false, None),
                    Tag::Heading { level, .. } => (heading(level), None, false, None),
                    Tag::BlockQuote(_) => ("blockquote", None, false, None),
                    Tag::CodeBlock(_) => ("pre", Some("code"), false, None),
                    Tag::List(Some(_)) => ("ol", None, false, None),
                    Tag::List(None) => ("ul", None, false, None),
                    Tag::Item => ("li", None, false, None),
                    Tag::Table(_) => ("table", None, false, None),
                    Tag::TableHead => {
                        in_head = true;
                        ("thead", Some("tr"), false, None)
                    }
                    Tag::TableRow => ("tr", None, false, None),
                    Tag::TableCell => (if in_head { "th" } else { "td" }, None, false, None),
                    Tag::Emphasis => ("em", None, false, None),
                    Tag::Strong => ("strong", None, false, None),
                    Tag::Strikethrough => ("del", None, false, None),
                    Tag::Link { dest_url, .. } => {
                        let href = link(&dest_url);
                        links += 1;
                        ("a", None, href.is_none(), href)
                    }
                    Tag::Image { dest_url, .. } => {
                        // Inside a link, the image is its alt text only.
                        let href = if links > 0 { None } else { link(&dest_url) };
                        links += 1;
                        ("a", None, href.is_none(), href)
                    }
                    Tag::HtmlBlock => ("p", None, false, None),
                    Tag::FootnoteDefinition(_) => ("div", None, false, None),
                    _ => ("span", None, true, None),
                };
                let mut element = Element::new(tag);
                if let Some(href) = href {
                    element.attrs.push(("href", href));
                }
                stack.push(Frame {
                    element,
                    wrap,
                    transparent,
                });
            }
            Event::End(end) => {
                match end {
                    TagEnd::TableHead => in_head = false,
                    TagEnd::Link | TagEnd::Image => links = links.saturating_sub(1),
                    _ => {}
                }
                if stack.len() < 2 {
                    continue;
                }
                let Some(frame) = stack.pop() else { continue };
                let mut element = frame.element;
                if let Some(wrap) = frame.wrap {
                    let mut inner = Element::new(wrap);
                    inner.children = std::mem::take(&mut element.children);
                    element.children.push(Node::Element(inner));
                }
                // An HTML block whose tags were all it held leaves nothing.
                let empty = element.children.iter().all(|child| match child {
                    Node::Text(text) => text.trim().is_empty(),
                    Node::Element(_) => false,
                });
                if end == TagEnd::HtmlBlock && empty {
                    continue;
                }
                if frame.transparent {
                    for child in element.children {
                        push(&mut stack, child);
                    }
                } else {
                    push(&mut stack, Node::Element(element));
                }
            }
            Event::Text(text) => push(&mut stack, Node::Text(text.into_string())),
            Event::Code(code) => {
                let mut element = Element::new("code");
                element.children.push(Node::Text(code.into_string()));
                push(&mut stack, Node::Element(element));
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                let text = stripper.strip(&html);
                if !text.is_empty() {
                    push(&mut stack, Node::Text(text));
                }
            }
            Event::SoftBreak => push(&mut stack, Node::Text("\n".into())),
            Event::HardBreak => push(&mut stack, Node::Element(Element::new("br"))),
            Event::Rule => push(&mut stack, Node::Element(Element::new("hr"))),
            Event::TaskListMarker(done) => {
                push(
                    &mut stack,
                    Node::Text(if done { "☑ " } else { "☐ " }.into()),
                );
            }
            Event::FootnoteReference(name) => {
                push(&mut stack, Node::Text(format!("[{name}]")));
            }
            Event::InlineMath(math) | Event::DisplayMath(math) => {
                push(&mut stack, Node::Text(math.into_string()));
            }
        }
    }
    // Anything left open is closed in order.
    while stack.len() > 1 {
        if let Some(frame) = stack.pop() {
            push(&mut stack, Node::Element(frame.element));
        }
    }
    stack
        .pop()
        .map(|frame| frame.element.children)
        .unwrap_or_default()
}

/// `text` with `&`, `<`, `>`, `"` and `'` escaped.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// Elements that have no closing tag.
fn is_void(tag: &str) -> bool {
    matches!(tag, "br" | "hr")
}

/// The tree as HTML, every text and attribute escaped.
pub fn to_html(nodes: &[Node]) -> String {
    let mut out = String::new();
    write_html(&mut out, nodes);
    out
}

fn write_html(out: &mut String, nodes: &[Node]) {
    for node in nodes {
        match node {
            Node::Text(text) => out.push_str(&escape(text)),
            Node::Element(element) => {
                out.push('<');
                out.push_str(element.tag);
                for (name, value) in &element.attrs {
                    out.push_str(&format!(" {name}=\"{}\"", escape(value)));
                    if *name == "href" && !value.starts_with('#') {
                        out.push_str(" rel=\"nofollow noopener\"");
                    }
                }
                out.push('>');
                if !is_void(element.tag) {
                    write_html(out, &element.children);
                    out.push_str("</");
                    out.push_str(element.tag);
                    out.push('>');
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn html(text: &str) -> String {
        to_html(&parse(text, absolute_only))
    }

    #[test]
    fn markdown_becomes_the_fixed_tags() {
        assert_eq!(
            html("# Title\n\nSome *em* and **strong** and `code`.\n\n- a\n- b\n"),
            "<h1>Title</h1><p>Some <em>em</em> and <strong>strong</strong> and \
             <code>code</code>.</p><ul><li>a</li><li>b</li></ul>"
        );
        assert_eq!(
            html("```rust\nfn main() {}\n```\n"),
            "<pre><code>fn main() {}\n</code></pre>"
        );
        assert_eq!(
            html("| a | b |\n|---|---|\n| 1 | 2 |\n"),
            "<table><thead><tr><th>a</th><th>b</th></tr></thead>\
             <tr><td>1</td><td>2</td></tr></table>"
        );
    }

    #[test]
    fn raw_html_is_never_markup() {
        let out = html("<script>alert(1)</script>\n\nhi <b onclick=\"x\">there</b>\n");
        assert!(!out.contains("<script"), "{out}");
        assert!(!out.contains("<b"), "{out}");
        assert!(!out.contains("onclick"), "{out}");
        assert!(out.contains("there"), "{out}");
        let header = html("<div align=\"center\">\n  <h1>Project</h1>\n</div>\n\nText\n");
        assert!(header.contains("Project"), "{header}");
        assert!(!header.contains("<div"), "{header}");
        assert!(!html("<!-- hidden -->\n\nshown\n").contains("hidden"));
        assert_eq!(
            html("Broken <script>alert(1)</script> thing"),
            "<p>Broken alert(1) thing</p>"
        );
    }

    #[test]
    fn only_the_policys_links_are_links() {
        assert_eq!(
            html("[a](https://x.test) [b](javascript:alert(1)) [c](rel/path)"),
            "<p><a href=\"https://x.test\" rel=\"nofollow noopener\">a</a> b c</p>"
        );
        assert_eq!(
            html("[ok](JavaScript:x)"),
            "<p>ok</p>",
            "the scheme is matched whatever its case"
        );
        assert_eq!(
            to_html(&parse("[c](docs/a.md)", |url| Some(format!("/base/{url}")))),
            "<p><a href=\"/base/docs/a.md\" rel=\"nofollow noopener\">c</a></p>"
        );
    }

    #[test]
    fn an_image_is_a_link_by_its_alt_text_and_never_inside_another() {
        assert_eq!(
            html("![logo](https://x.test/l.png)"),
            "<p><a href=\"https://x.test/l.png\" rel=\"nofollow noopener\">logo</a></p>"
        );
        assert_eq!(
            html("[![CI](https://x.test/b.svg)](https://x.test/ci)"),
            "<p><a href=\"https://x.test/ci\" rel=\"nofollow noopener\">CI</a></p>"
        );
    }

    #[test]
    fn text_is_escaped() {
        assert_eq!(html("a & b < c"), "<p>a &amp; b &lt; c</p>");
        assert_eq!(html("`<b>`"), "<p><code>&lt;b&gt;</code></p>");
    }

    #[test]
    fn a_scheme_is_found_before_any_path() {
        assert_eq!(scheme("https://x").as_deref(), Some("https"));
        assert_eq!(scheme("a/b:c"), None);
        assert_eq!(scheme(":x"), None);
        assert_eq!(scheme("plain"), None);
    }
}
