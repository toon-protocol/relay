//! Markdown for the app UIs: a README in the browser, an issue or a comment
//! on the server.
//!
//! A document becomes a tree of [`Node`]s whose tags come from a fixed set
//! and whose links and images go where the caller decides. [`to_html`]
//! writes the tree as escaped HTML for a server-rendered page; the browser
//! half builds the same tree as DOM.
//!
//! [`parse`] reads an event's text (an issue, a comment, a post): raw HTML is
//! never markup, its tags dropped and the text between them kept, and an
//! image is not loaded but shown as a link to itself by its alt text.
//! [`parse_rich`] reads a repository's Markdown as GitHub draws it (ADR
//! 0009): the raw tags in a fixed list are kept with presentational
//! attributes only, and images are drawn.

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
    /// From a fixed set of names: `href` and `src` only as the caller's
    /// policies returned them, and the presentational ones [`parse_rich`]
    /// keeps.
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
    /// Opened by a raw HTML tag: the name of the tag that closes it. A raw
    /// element still open when its Markdown parent ends is closed with it.
    closes: Option<&'static str>,
    /// An image: its children's text becomes its `alt` and it keeps none.
    alt: bool,
}

impl Frame {
    fn new(element: Element) -> Self {
        Self {
            element,
            wrap: None,
            transparent: false,
            closes: None,
            alt: false,
        }
    }
}

/// One piece of a raw HTML run.
#[derive(Debug, PartialEq, Eq)]
enum Piece {
    Text(String),
    Open {
        name: String,
        attrs: Vec<(String, String)>,
    },
    Close(String),
}

/// Raw HTML runs read as tags and the text between them. A tag or comment
/// left open runs on into the next run.
#[derive(Default)]
struct Tokenizer {
    pending: String,
    in_tag: bool,
    quote: Option<char>,
    in_comment: bool,
}

impl Tokenizer {
    fn feed(&mut self, html: &str) -> Vec<Piece> {
        let mut pieces = Vec::new();
        let mut text = String::new();
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
                let mut end = None;
                for (at, character) in rest.char_indices() {
                    match self.quote {
                        Some(quote) if character == quote => self.quote = None,
                        Some(_) => {}
                        None if character == '"' || character == '\'' => {
                            self.quote = Some(character);
                        }
                        None if character == '>' => {
                            end = Some(at);
                            break;
                        }
                        None => {}
                    }
                }
                match end {
                    Some(at) => {
                        self.pending.push_str(&rest[..at]);
                        rest = &rest[at + 1..];
                        self.in_tag = false;
                        if let Some(piece) = tag(&std::mem::take(&mut self.pending)) {
                            if !text.is_empty() {
                                pieces
                                    .push(Piece::Text(decode_entities(&std::mem::take(&mut text))));
                            }
                            pieces.push(piece);
                        }
                    }
                    None => {
                        self.pending.push_str(rest);
                        rest = "";
                    }
                }
            } else if let Some(start) = rest.find('<') {
                text.push_str(&rest[..start]);
                if rest[start..].starts_with("<!--") {
                    self.in_comment = true;
                    rest = &rest[start + 4..];
                } else {
                    self.in_tag = true;
                    rest = &rest[start + 1..];
                }
            } else {
                text.push_str(rest);
                rest = "";
            }
        }
        if !text.is_empty() {
            pieces.push(Piece::Text(decode_entities(&text)));
        }
        pieces
    }
}

/// The tag between `<` and `>`, or none for a declaration or anything else
/// that is not one.
fn tag(inner: &str) -> Option<Piece> {
    let inner = inner.trim();
    if let Some(name) = inner.strip_prefix('/') {
        return Some(Piece::Close(name.trim().to_ascii_lowercase()));
    }
    let inner = inner.strip_suffix('/').unwrap_or(inner);
    let end = inner.find(char::is_whitespace).unwrap_or(inner.len());
    let name = inner[..end].to_ascii_lowercase();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric()) {
        return None;
    }
    Some(Piece::Open {
        name,
        attrs: attributes(&inner[end..]),
    })
}

/// `name="value"` pairs, lowercased names, values unquoted and decoded.
fn attributes(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text.trim_start();
    while !rest.is_empty() {
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '=')
            .unwrap_or(rest.len());
        let name = rest[..end].to_ascii_lowercase();
        rest = rest[end..].trim_start();
        let mut value = String::new();
        if let Some(after) = rest.strip_prefix('=') {
            let after = after.trim_start();
            match after.chars().next() {
                Some(quote @ ('"' | '\'')) => {
                    let body = &after[1..];
                    let close = body.find(quote).unwrap_or(body.len());
                    value = body[..close].to_owned();
                    rest = body.get(close + 1..).unwrap_or_default();
                }
                _ => {
                    let close = after.find(char::is_whitespace).unwrap_or(after.len());
                    value = after[..close].to_owned();
                    rest = &after[close..];
                }
            }
        }
        if !name.is_empty() {
            out.push((name, decode_entities(&value)));
        }
        rest = rest.trim_start();
    }
    out
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

/// The raw HTML tags a document read [`parse_rich`] keeps, as GitHub's
/// sanitizer keeps them. Any other tag is dropped and its text kept.
const TAGS: &[&str] = &[
    "a",
    "b",
    "blockquote",
    "br",
    "code",
    "dd",
    "del",
    "details",
    "div",
    "dl",
    "dt",
    "em",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "hr",
    "i",
    "img",
    "ins",
    "kbd",
    "li",
    "ol",
    "p",
    "picture",
    "pre",
    "s",
    "samp",
    "small",
    "span",
    "strong",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "tfoot",
    "th",
    "thead",
    "tr",
    "ul",
    "var",
];

fn is_number(value: &str) -> bool {
    !value.is_empty() && value.len() <= 6 && value.bytes().all(|b| b.is_ascii_digit())
}

/// A raw tag's attribute as it is kept, if it is: presentational ones with
/// plain values only. `href` and `src` go through the caller's policies.
fn kept(tag: &str, name: &str, value: &str) -> Option<&'static str> {
    let value = value.trim().to_ascii_lowercase();
    match name {
        "align" if matches!(value.as_str(), "left" | "center" | "right" | "justify") => {
            Some("align")
        }
        "width" | "height"
            if matches!(tag, "img" | "td" | "th" | "table")
                && (is_number(&value)
                    || value.strip_suffix('%').is_some_and(is_number)
                    || value.strip_suffix("px").is_some_and(is_number)) =>
        {
            Some(if name == "width" { "width" } else { "height" })
        }
        "colspan" | "rowspan" if matches!(tag, "td" | "th") && is_number(&value) => {
            Some(if name == "colspan" {
                "colspan"
            } else {
                "rowspan"
            })
        }
        "alt" if tag == "img" => Some("alt"),
        "title" => Some("title"),
        "open" if tag == "details" => Some("open"),
        _ => None,
    }
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
/// goes: `None` leaves its text unlinked. Raw HTML is never markup and an
/// image is a link to itself by its alt text.
pub fn parse(text: &str, link: impl Fn(&str) -> Option<String>) -> Vec<Node> {
    read(text, &link, None)
}

/// Reads `text` as GitHub draws a repository's Markdown: as [`parse`] does,
/// but the raw HTML tags in [`TAGS`] are kept with their presentational
/// attributes (a README's centred header), and an image is drawn from the
/// `src` that `image` decides for it (`None` draws it as [`parse`] does).
pub fn parse_rich(
    text: &str,
    link: impl Fn(&str) -> Option<String>,
    image: impl Fn(&str) -> Option<String>,
) -> Vec<Node> {
    read(text, &link, Some(&image))
}

/// Where a link or image goes, or `None` for nowhere.
type Policy<'a> = &'a dyn Fn(&str) -> Option<String>;

fn push(stack: &mut [Frame], node: Node) {
    if let Some(frame) = stack.last_mut() {
        if let (Node::Text(text), Some(Node::Text(last))) =
            (&node, frame.element.children.last_mut())
        {
            last.push_str(text);
            return;
        }
        frame.element.children.push(node);
    }
}

fn text_of(nodes: &[Node]) -> String {
    nodes
        .iter()
        .map(|node| match node {
            Node::Text(text) => text.clone(),
            Node::Element(element) => text_of(&element.children),
        })
        .collect()
}

/// Closes the top frame into its parent. `html_block` drops one whose
/// tags were all it held.
fn close(stack: &mut Vec<Frame>, html_block: bool) {
    if stack.len() < 2 {
        return;
    }
    let Some(frame) = stack.pop() else { return };
    let mut element = frame.element;
    if let Some(wrap) = frame.wrap {
        let mut inner = Element::new(wrap);
        inner.children = std::mem::take(&mut element.children);
        element.children.push(Node::Element(inner));
    }
    if frame.alt {
        let alt = text_of(&std::mem::take(&mut element.children));
        element.attrs.push(("alt", alt));
    }
    let empty = element.children.iter().all(|child| match child {
        Node::Text(text) => text.trim().is_empty(),
        Node::Element(_) => false,
    });
    if html_block && empty {
        return;
    }
    if frame.transparent {
        for child in element.children {
            push(stack, child);
        }
    } else {
        push(stack, Node::Element(element));
    }
}

fn in_link(stack: &[Frame]) -> bool {
    stack.iter().any(|frame| frame.element.tag == "a")
}

/// One raw HTML piece of a [`parse_rich`] document, into the tree.
fn raw(stack: &mut Vec<Frame>, piece: Piece, link: Policy<'_>, image: Policy<'_>) {
    match piece {
        Piece::Text(text) => {
            if !text.is_empty() {
                push(stack, Node::Text(text));
            }
        }
        Piece::Close(name) => {
            let mut at = None;
            for (index, frame) in stack.iter().enumerate().skip(1).rev() {
                match frame.closes {
                    Some(closes) if closes == name => {
                        at = Some(index);
                        break;
                    }
                    Some(_) => {}
                    None => break,
                }
            }
            if let Some(at) = at {
                while stack.len() > at {
                    close(stack, false);
                }
            }
        }
        Piece::Open { name, attrs } => {
            let Some(tag) = TAGS.iter().copied().find(|tag| *tag == name) else {
                return;
            };
            let mut element = Element::new(tag);
            for (name, value) in &attrs {
                if let Some(name) = kept(tag, name, value) {
                    element.attrs.push((name, value.trim().to_owned()));
                }
            }
            let attr = |wanted: &str| {
                attrs
                    .iter()
                    .find(|(name, _)| name == wanted)
                    .map(|(_, value)| value.trim())
            };
            match tag {
                "img" => match attr("src").and_then(image) {
                    Some(src) => {
                        element.attrs.insert(0, ("src", src));
                        push(stack, Node::Element(element));
                    }
                    None => {
                        if let Some(alt) = attr("alt").filter(|alt| !alt.is_empty()) {
                            push(stack, Node::Text(alt.to_owned()));
                        }
                    }
                },
                "br" | "hr" => push(stack, Node::Element(element)),
                _ => {
                    let mut frame = Frame::new(element);
                    frame.closes = Some(tag);
                    if tag == "a" {
                        let href = if in_link(stack) {
                            None
                        } else {
                            attr("href").and_then(link)
                        };
                        match href {
                            Some(href) => frame.element.attrs.insert(0, ("href", href)),
                            // A link that goes nowhere is its text alone.
                            None => {
                                frame.element.tag = "span";
                                frame.transparent = true;
                            }
                        }
                    }
                    stack.push(frame);
                }
            }
        }
    }
}

fn read(text: &str, link: Policy<'_>, image: Option<Policy<'_>>) -> Vec<Node> {
    let options = Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_GFM;
    let mut root = Frame::new(Element::new("div"));
    root.transparent = true;
    let mut stack: Vec<Frame> = vec![root];
    let mut tokenizer = Tokenizer::default();
    let mut in_head = false;
    for event in Parser::new_ext(text, options) {
        match event {
            // A rich document's HTML block is its tags, not a paragraph.
            Event::Start(Tag::HtmlBlock) | Event::End(TagEnd::HtmlBlock) if image.is_some() => {}
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
                        ("a", None, href.is_none(), href)
                    }
                    Tag::Image {
                        dest_url, title, ..
                    } => {
                        if let Some(src) = image.and_then(|image| image(&dest_url)) {
                            let mut element = Element::new("img");
                            element.attrs.push(("src", src));
                            if !title.is_empty() {
                                element.attrs.push(("title", title.into_string()));
                            }
                            let mut frame = Frame::new(element);
                            frame.alt = true;
                            stack.push(frame);
                            continue;
                        }
                        // Inside a link, the image is its alt text only.
                        let href = if in_link(&stack) {
                            None
                        } else {
                            link(&dest_url)
                        };
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
                let mut frame = Frame::new(element);
                frame.wrap = wrap;
                frame.transparent = transparent;
                stack.push(frame);
            }
            Event::End(end) => {
                if end == TagEnd::TableHead {
                    in_head = false;
                }
                // Raw elements left open inside this one end with it.
                while stack.len() > 1 && stack.last().is_some_and(|f| f.closes.is_some()) {
                    close(&mut stack, false);
                }
                close(&mut stack, end == TagEnd::HtmlBlock);
            }
            Event::Text(text) => push(&mut stack, Node::Text(text.into_string())),
            Event::Code(code) => {
                let mut element = Element::new("code");
                element.children.push(Node::Text(code.into_string()));
                push(&mut stack, Node::Element(element));
            }
            Event::Html(html) | Event::InlineHtml(html) => {
                for piece in tokenizer.feed(&html) {
                    match image {
                        Some(image) => raw(&mut stack, piece, link, image),
                        None => {
                            if let Piece::Text(text) = piece {
                                push(&mut stack, Node::Text(text));
                            }
                        }
                    }
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
        close(&mut stack, false);
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
    matches!(tag, "br" | "hr" | "img")
}

/// Where a text node sits in the tree, for a caller that draws text itself.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Place {
    /// Inside a link: another link here would nest one in the other.
    pub in_link: bool,
    /// Inside inline code or a code block: the text is code, not prose.
    pub in_code: bool,
}

/// The tree as HTML, every text and attribute escaped.
pub fn to_html(nodes: &[Node]) -> String {
    to_html_with(nodes, &mut |text, _| escape(text))
}

/// The tree as HTML, each text node written by `text`, which is told where
/// the node sits. Its contract: what it returns is HTML that holds the
/// node's text escaped (it may wrap parts of it in elements of its own, such
/// as a mention's link or a highlight's mark). Tags and attributes are
/// written as [`to_html`] writes them.
pub fn to_html_with(nodes: &[Node], text: &mut impl FnMut(&str, Place) -> String) -> String {
    let mut out = String::new();
    write_html(&mut out, nodes, Place::default(), text);
    out
}

fn write_html(
    out: &mut String,
    nodes: &[Node],
    place: Place,
    text: &mut impl FnMut(&str, Place) -> String,
) {
    for node in nodes {
        match node {
            Node::Text(words) => out.push_str(&text(words, place)),
            Node::Element(element) => {
                out.push('<');
                out.push_str(element.tag);
                for (name, value) in &element.attrs {
                    out.push_str(&format!(" {name}=\"{}\"", escape(value)));
                    if *name == "href" && !value.starts_with('#') {
                        out.push_str(" rel=\"nofollow noopener\"");
                    }
                }
                if element.tag == "img" {
                    out.push_str(" referrerpolicy=\"no-referrer\"");
                }
                out.push('>');
                if !is_void(element.tag) {
                    let inner = Place {
                        in_link: place.in_link || element.tag == "a",
                        in_code: place.in_code || matches!(element.tag, "code" | "pre"),
                    };
                    write_html(out, &element.children, inner, text);
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
    fn a_text_writer_is_told_whether_its_text_is_a_link_or_code() {
        let nodes = parse(
            "plain [linked](https://x.test) `code`\n\n```\nblock\n```\n",
            absolute_only,
        );
        let mut seen = Vec::new();
        let out = to_html_with(&nodes, &mut |text, place| {
            seen.push((text.to_owned(), place));
            escape(&text.to_uppercase())
        });
        assert!(out.contains("<p>PLAIN <a href"), "{out}");
        assert!(out.contains(">LINKED</a>"), "{out}");
        assert!(out.contains("<code>CODE</code>"), "{out}");
        let place = |text: &str| seen.iter().find(|(t, _)| t == text).map(|(_, p)| *p);
        assert_eq!(place("plain "), Some(Place::default()));
        let link = Place {
            in_link: true,
            in_code: false,
        };
        assert_eq!(place("linked"), Some(link));
        let code = Place {
            in_link: false,
            in_code: true,
        };
        assert_eq!(place("block\n"), Some(code));
        assert_eq!(to_html(&nodes), to_html_with(&nodes, &mut |t, _| escape(t)));
    }

    fn rich(text: &str) -> String {
        to_html(&parse_rich(text, absolute_only, |src| {
            src.starts_with("https://").then(|| src.to_owned())
        }))
    }

    #[test]
    fn a_readmes_centred_header_keeps_its_tags() {
        let out = rich(
            "<div align=\"center\">\n\n<img src=\"https://x.test/logo.png\" alt=\"Logo\" \
             width=\"160\">\n\n# Title\n\n[![CI](https://x.test/ci.svg)](https://x.test/ci)\n\n\
             </div>\n\nAfter\n",
        );
        assert_eq!(
            out,
            "<div align=\"center\">\n<img src=\"https://x.test/logo.png\" alt=\"Logo\" \
             width=\"160\" referrerpolicy=\"no-referrer\">\n<h1>Title</h1>\
             <p><a href=\"https://x.test/ci\" rel=\"nofollow noopener\"><img \
             src=\"https://x.test/ci.svg\" alt=\"CI\" referrerpolicy=\"no-referrer\"></a></p>\
             </div>\n<p>After</p>"
        );
    }

    #[test]
    fn a_rich_document_keeps_only_listed_tags_and_plain_attributes() {
        let out = rich(
            "<p align=\"center\" style=\"color:red\" onclick=\"x()\">hi <b class=\"c\">there</b> \
             <script>alert(1)</script><iframe src=\"https://x.test\"></iframe></p>\n",
        );
        assert_eq!(
            out, "<p align=\"center\">hi <b>there</b> alert(1)</p>\n",
            "only the listed tags and attributes survive"
        );
        assert_eq!(rich("<p align=\"evil\">x</p>\n"), "<p>x</p>\n");
        assert_eq!(
            rich("<img src=\"https://x.test/a.png\" width=\"50%\" height=\"1;x\">\n"),
            "<img src=\"https://x.test/a.png\" width=\"50%\" referrerpolicy=\"no-referrer\">\n"
        );
        assert_eq!(
            rich("<a href=\"javascript:alert(1)\">no</a> <a href=\"https://x.test\">yes</a>\n"),
            "<p>no <a href=\"https://x.test\" rel=\"nofollow noopener\">yes</a></p>"
        );
        assert_eq!(
            rich("<details open><summary>More</summary>\n\nBody\n\n</details>\n"),
            "<details open=\"\"><summary>More</summary>\n<p>Body</p></details>\n"
        );
    }

    #[test]
    fn an_image_the_policy_refuses_is_drawn_as_parse_draws_it() {
        assert_eq!(
            rich("![logo](http://x.test/l.png)"),
            "<p><a href=\"http://x.test/l.png\" rel=\"nofollow noopener\">logo</a></p>"
        );
        assert_eq!(rich("<img src=\"data:x\" alt=\"Logo\">\n"), "Logo\n");
    }

    #[test]
    fn raw_tags_left_open_close_with_their_parent_and_stray_closes_are_dropped() {
        assert_eq!(
            rich("a <b>bold\n\nnext</b> c\n"),
            "<p>a <b>bold</b></p><p>next c</p>"
        );
        assert_eq!(rich("</div>\n\ntext\n"), "\n<p>text</p>");
        assert_eq!(
            rich("<div\n  align=\"center\">\n\nx\n\n</div>\n"),
            "<div align=\"center\">\n<p>x</p></div>\n",
            "a tag may run over lines"
        );
    }

    #[test]
    fn attributes_are_read_quoted_unquoted_and_bare() {
        assert_eq!(
            attributes(" a=\"1 2\" b='3' c=4 d e = \"&amp;\""),
            [
                ("a".into(), "1 2".into()),
                ("b".into(), "3".into()),
                ("c".into(), "4".into()),
                ("d".into(), String::new()),
                ("e".into(), "&".into()),
            ]
        );
    }

    #[test]
    fn a_scheme_is_found_before_any_path() {
        assert_eq!(scheme("https://x").as_deref(), Some("https"));
        assert_eq!(scheme("a/b:c"), None);
        assert_eq!(scheme(":x"), None);
        assert_eq!(scheme("plain"), None);
    }
}
