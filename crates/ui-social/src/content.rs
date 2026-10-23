//! A note's text as HTML: plain text, never markup, with its links and its
//! `nostr:` mentions (NIP-21, NIP-27) drawn as links. A mention of a repo
//! (an `naddr` of kind 30617) is collected so the page can unfurl it into a
//! repo card below the text. Pure functions, no I/O.

use std::collections::HashMap;

use nostr::key::PublicKey;
use nostr::nips::nip21::Nip21;
use nostr::parser::{NostrParser, Token};
use relay_ui::chrome::{escape, npub};
use ui_nip34::repo::RepoAddress;

use crate::PREFIX;

/// A note's text, drawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Text {
    /// The HTML, for an element whose white space is kept.
    pub(crate) html: String,
    /// The repos it mentions, once each, in order.
    pub(crate) repos: Vec<RepoAddress>,
}

/// The keys `text` mentions, so their names can be read before drawing it.
pub(crate) fn mentioned_keys(text: &str) -> Vec<PublicKey> {
    let mut keys: Vec<PublicKey> = Vec::new();
    for token in NostrParser::new().parse(text) {
        let key = match token {
            Token::Nostr(Nip21::Pubkey(key)) => key,
            Token::Nostr(Nip21::Profile(profile)) => profile.public_key,
            _ => continue,
        };
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    keys
}

/// `text` as escaped HTML. A URL in `hidden` (media the page shows from the
/// note's `imeta` tags) is left out; another `http(s)` URL is a link that
/// passes nothing on. `names` are the display names of mentioned keys.
pub(crate) fn render(text: &str, hidden: &[String], names: &HashMap<PublicKey, String>) -> Text {
    let mut html = String::with_capacity(text.len());
    let mut repos: Vec<RepoAddress> = Vec::new();
    for token in NostrParser::new().parse(text) {
        match token {
            Token::Text(text) => html.push_str(&escape(text)),
            Token::Hashtag(tag) => {
                html.push_str(&format!("<span class=\"muted\">#{}</span>", escape(tag)))
            }
            Token::LineBreak => html.push('\n'),
            Token::Whitespace => html.push(' '),
            Token::Url(url) => {
                let url = url.as_str();
                if hidden.iter().any(|shown| shown == url) {
                    continue;
                }
                if url.starts_with("https://") || url.starts_with("http://") {
                    html.push_str(&format!(
                        "<a href=\"{0}\" rel=\"nofollow ugc noopener noreferrer\">{0}</a>",
                        escape(url)
                    ));
                } else {
                    html.push_str(&escape(url));
                }
            }
            Token::Nostr(mention) => html.push_str(&mention_html(mention, names, &mut repos)),
        }
    }
    Text { html, repos }
}

/// One `nostr:` mention, drawn; a repo is also added to `repos`.
fn mention_html(
    mention: Nip21,
    names: &HashMap<PublicKey, String>,
    repos: &mut Vec<RepoAddress>,
) -> String {
    let person = |key: PublicKey| {
        let shown = names
            .get(&key)
            .cloned()
            .unwrap_or_else(|| format!("{}…", &npub(&key)[..12]));
        format!(
            "<a class=\"strong\" href=\"{PREFIX}/following/{0}\" title=\"{0}\">@{1}</a>",
            npub(&key),
            escape(&shown)
        )
    };
    let note = |id: nostr::event::EventId| {
        let hex = id.to_hex();
        format!(
            "<a class=\"mono\" href=\"{PREFIX}/e/{hex}\">note:{}</a>",
            &hex[..8]
        )
    };
    match mention {
        Nip21::Pubkey(key) => person(key),
        Nip21::Profile(profile) => person(profile.public_key),
        Nip21::EventId(id) => note(id),
        Nip21::Event(event) => note(event.event_id),
        Nip21::Coordinate(coordinate) => {
            let address = RepoAddress::new(coordinate.public_key, &coordinate.identifier)
                .filter(|_| coordinate.kind.as_u16() == ui_nip34::repo::ANNOUNCEMENT);
            match address {
                Some(address) => {
                    let link = format!(
                        "<a class=\"mono\" href=\"{PREFIX}/r/{}\">{}</a>",
                        escape(&address.path_segments()),
                        escape(address.d())
                    );
                    if !repos.contains(&address) {
                        repos.push(address);
                    }
                    link
                }
                None => match crate::article::ArticleAddress::from_coordinate(&coordinate) {
                    Some(article) => format!(
                        "<a class=\"mono\" href=\"{}\">article:{}</a>",
                        escape(&article.path()),
                        escape(article.d())
                    ),
                    None => format!(
                        "<span class=\"mono muted\">{}:{}</span>",
                        coordinate.kind.as_u16(),
                        escape(&coordinate.identifier)
                    ),
                },
            }
        }
    }
}

/// The first `limit` characters of `text`, and an ellipsis if it went on.
pub(crate) fn clip(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_owned(),
    }
}

/// The first non-empty line of `text`, clipped: a post's title when it
/// names none.
pub(crate) fn first_line(text: &str, limit: usize) -> String {
    clip(
        text.lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or(""),
        limit,
    )
}

/// The first sentence of the first non-empty line, clipped: a launch's
/// headline when it names none.
pub(crate) fn headline(text: &str, limit: usize) -> String {
    let line = first_line(text, usize::MAX);
    let end = line
        .char_indices()
        .find(|(index, c)| {
            matches!(c, '.' | '!' | '?')
                && line[index + c.len_utf8()..].starts_with(char::is_whitespace)
        })
        .map_or(line.len(), |(index, c)| index + c.len_utf8());
    clip(line[..end].trim_end_matches('.'), limit)
}

#[cfg(test)]
mod tests {
    use nostr::event::Kind;
    use nostr::key::Keys;
    use nostr::nips::nip01::Coordinate;
    use nostr::nips::nip19::{Nip19Coordinate, ToBech32};

    use super::*;

    fn naddr(kind: u16, key: PublicKey, d: &str) -> String {
        Nip19Coordinate::new(
            Coordinate::new(Kind::from(kind), key).identifier(d),
            Vec::<nostr::types::RelayUrl>::new(),
        )
        .to_bech32()
        .unwrap()
    }

    #[test]
    fn a_repo_mention_is_collected_and_linked() {
        let owner = Keys::generate().public_key();
        let text = format!(
            "Try nostr:{} and nostr:{} twice: nostr:{}",
            naddr(30_617, owner, "gitsail"),
            naddr(30_023, owner, "post"),
            naddr(30_617, owner, "gitsail")
        );
        let drawn = render(&text, &[], &HashMap::new());
        assert_eq!(
            drawn.repos,
            vec![RepoAddress::new(owner, "gitsail").unwrap()]
        );
        assert!(
            drawn
                .html
                .contains(&format!("href=\"/social/r/{}/gitsail\"", npub(&owner)))
        );
        assert!(
            drawn.html.contains(&format!(
                "href=\"{}\">article:post</a>",
                crate::article::ArticleAddress::new(owner, "post").path()
            )),
            "an article mention links to its reader"
        );
    }

    #[test]
    fn markup_in_a_note_is_text_and_hidden_media_is_left_out() {
        let drawn = render(
            "<script>alert(1)</script> see https://cdn.test/a.png and https://x.test/\nok",
            &["https://cdn.test/a.png".to_owned()],
            &HashMap::new(),
        );
        assert!(drawn.html.contains("&lt;script&gt;"));
        assert!(!drawn.html.contains("<script"));
        assert!(!drawn.html.contains("cdn.test"));
        assert!(
            drawn
                .html
                .contains("<a href=\"https://x.test/\" rel=\"nofollow")
        );
        assert!(drawn.html.contains('\n'));
    }

    #[test]
    fn a_mentioned_person_is_shown_by_name() {
        let key = Keys::generate().public_key();
        let text = format!("hi nostr:{}", npub(&key));
        assert_eq!(mentioned_keys(&text), vec![key]);
        let names = HashMap::from([(key, "Ada <3".to_owned())]);
        assert!(render(&text, &[], &names).html.contains("@Ada &lt;3"));
    }

    #[test]
    fn clipping_counts_characters_not_bytes() {
        assert_eq!(clip("héllo", 2), "hé…");
        assert_eq!(clip("hi", 5), "hi");
        assert_eq!(first_line("\n  \n  Shipped it  \nmore", 50), "Shipped it");
        assert_eq!(
            headline(
                "Shipped tidepool: charts. Vibe-coded v1.2 in a weekend.",
                80
            ),
            "Shipped tidepool: charts"
        );
        assert_eq!(headline("v1.2 is out", 80), "v1.2 is out");
    }
}
