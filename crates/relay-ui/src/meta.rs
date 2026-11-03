//! What a page tells a link unfurler (#314): Open Graph and X card tags in
//! the shell's `<head>`, so a `/social` or `/repo` link posted elsewhere
//! shows a title, a description and the card image the relay draws
//! (`ui-preview`, ADR 0006).
//!
//! Open Graph wants absolute URLs, and the relay only knows the origin it is
//! reached at when the operator says it: [`PublicUrl`]. Without one a page
//! still carries its title and description, and leaves out `og:url` and
//! every image tag rather than name a relative image a crawler would drop.

use crate::chrome::{App, Apps, escape};

/// The most characters of a description a page states.
pub const DESCRIPTION_CHARS: usize = 200;

/// The `https` origin visitors reach the relay's read port at
/// (`TOON_UI_PUBLIC_URL`), with no path: `https://relay.example`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublicUrl {
    origin: String,
}

impl PublicUrl {
    /// `text` as an origin: `https://`, a host (and port), and at most a
    /// trailing slash; none for anything else, `http` included. A pure
    /// function.
    pub fn parse(text: &str) -> Option<Self> {
        let authority = text.trim().strip_prefix("https://")?;
        let authority = authority.strip_suffix('/').unwrap_or(authority);
        Self::authority(authority)
    }

    /// The origin a relay URL (`TOON_UI_RELAY_URL`) implies when it is a
    /// secure one: `wss://host/x` and `https://host/x` are `https://host`.
    /// A `ws://` or `http://` URL implies none: a crawler fetches images
    /// over `https`. A pure function.
    pub fn from_relay_url(url: &str) -> Option<Self> {
        let rest = url
            .trim()
            .strip_prefix("wss://")
            .or_else(|| url.trim().strip_prefix("https://"))?;
        let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
        Self::authority(authority)
    }

    fn authority(authority: &str) -> Option<Self> {
        let (host, port) = match authority.rsplit_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        let host_ok = !host.is_empty()
            && host
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-.".contains(&b))
            && !host.starts_with(['.', '-']);
        let port_ok = port.is_none_or(|port| {
            !port.is_empty() && port.len() <= 5 && port.bytes().all(|b| b.is_ascii_digit())
        });
        (host_ok && port_ok).then(|| Self {
            origin: format!("https://{}", authority.to_ascii_lowercase()),
        })
    }

    /// `https://host[:port]`.
    pub fn origin(&self) -> &str {
        &self.origin
    }

    /// `host[:port]`, as a card's footer names it.
    pub fn host(&self) -> &str {
        self.origin.trim_start_matches("https://")
    }

    /// `path` (which starts with `/`) on this origin.
    pub fn absolute(&self, path: &str) -> String {
        format!("{}{path}", self.origin)
    }
}

/// What a page says about itself to an unfurler. [`Preview::default`] is a
/// page with no card of its own: the app's site card.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Preview {
    title: Option<String>,
    description: Option<String>,
    kind: Option<&'static str>,
    path: Option<String>,
    image: Option<(String, String)>,
}

impl Preview {
    /// A page about one thing, called `title`, described by `description`
    /// (plain text, any length: it is cut to [`DESCRIPTION_CHARS`]).
    pub fn new(title: impl Into<String>, description: impl Into<String>) -> Self {
        Self {
            title: Some(title.into()),
            description: Some(description.into()),
            ..Self::default()
        }
    }

    /// Its Open Graph type: `profile`, `article`; `website` otherwise.
    pub fn kind(mut self, kind: &'static str) -> Self {
        self.kind = Some(kind);
        self
    }

    /// The page's canonical path, for `og:url`.
    pub fn at(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    /// Its card image at `path` on this relay, described by `alt`.
    pub fn image(mut self, path: impl Into<String>, alt: impl Into<String>) -> Self {
        self.image = Some((path.into(), alt.into()));
        self
    }

    /// The `<meta>` tags for a page titled `page_title` drawn by `app`,
    /// among `apps`. A pure function.
    pub fn tags(&self, page_title: &str, app: App, apps: &Apps) -> String {
        let title = self.title.as_deref().unwrap_or(page_title);
        let description = summary(
            self.description.as_deref().unwrap_or_else(|| tagline(app)),
            DESCRIPTION_CHARS,
        );
        let mut tags = vec![
            ("property", "og:site_name", "TOON".to_owned()),
            (
                "property",
                "og:type",
                self.kind.unwrap_or("website").to_owned(),
            ),
            ("property", "og:title", title.to_owned()),
            ("property", "og:description", description.clone()),
            ("name", "description", description.clone()),
        ];
        let origin = apps.public_url();
        if let (Some(origin), Some(path)) = (origin, &self.path) {
            tags.push(("property", "og:url", origin.absolute(path)));
        }
        let card = match origin {
            Some(origin) => {
                let (path, alt) = self.image.clone().unwrap_or_else(|| {
                    (
                        site_image(app),
                        format!("TOON {}: {}", app.label(), tagline(app)),
                    )
                });
                let image = origin.absolute(&path);
                tags.extend([
                    ("property", "og:image", image.clone()),
                    ("property", "og:image:type", "image/png".to_owned()),
                    ("property", "og:image:width", "1200".to_owned()),
                    ("property", "og:image:height", "630".to_owned()),
                    ("property", "og:image:alt", alt.clone()),
                    ("name", "twitter:image", image),
                    ("name", "twitter:image:alt", alt),
                ]);
                "summary_large_image"
            }
            None => "summary",
        };
        tags.extend([
            ("name", "twitter:card", card.to_owned()),
            ("name", "twitter:title", title.to_owned()),
            ("name", "twitter:description", description),
        ]);
        tags.iter()
            .map(|(attribute, name, content)| {
                format!(
                    "<meta {attribute}=\"{name}\" content=\"{}\">",
                    escape(content)
                )
            })
            .collect()
    }
}

/// What `app` is, for a page with nothing more specific to say.
pub fn tagline(app: App) -> &'static str {
    match app {
        App::Repo => "Git repositories over Nostr (NIP-34), read from this TOON relay.",
        App::Social => {
            "Builders sharing repos, launches and devlogs, read from this TOON relay's Nostr events."
        }
    }
}

/// The path of `app`'s site card.
pub fn site_image(app: App) -> String {
    format!("{}/og/site.png", app.prefix())
}

/// `text` as one line of plain text: white space collapsed, control
/// characters left out, cut to `max` characters with `…` where it is cut.
/// A pure function.
pub fn summary(text: &str, max: usize) -> String {
    let words: Vec<&str> = text.split_whitespace().collect();
    let joined: String = words
        .join(" ")
        .chars()
        .filter(|character| !character.is_control())
        .collect();
    if joined.chars().count() <= max {
        return joined;
    }
    let cut: String = joined.chars().take(max.saturating_sub(1)).collect();
    let cut = match cut.rfind(' ') {
        Some(space) if space > cut.len() / 2 => &cut[..space],
        _ => cut.as_str(),
    };
    format!("{}…", cut.trim_end_matches([' ', ',', '.', ';', ':']))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_public_url_is_an_https_origin_and_nothing_else() {
        let url = PublicUrl::parse("https://Relay.Example/").unwrap();
        assert_eq!(url.origin(), "https://relay.example");
        assert_eq!(url.host(), "relay.example");
        assert_eq!(url.absolute("/social"), "https://relay.example/social");
        assert_eq!(
            PublicUrl::parse("https://relay.example:8443")
                .unwrap()
                .origin(),
            "https://relay.example:8443"
        );
        for bad in [
            "http://relay.example",
            "https://",
            "https://relay.example/x",
            "https://relay.example?x",
            "https://user@relay.example",
            "https://relay.example:port",
            "relay.example",
            "https://<b>",
        ] {
            assert_eq!(PublicUrl::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_secure_relay_url_implies_its_https_origin() {
        assert_eq!(
            PublicUrl::from_relay_url("wss://relay.toonprotocol.dev")
                .unwrap()
                .origin(),
            "https://relay.toonprotocol.dev"
        );
        assert_eq!(
            PublicUrl::from_relay_url("https://relay.example:444/path")
                .unwrap()
                .origin(),
            "https://relay.example:444"
        );
        assert_eq!(PublicUrl::from_relay_url("ws://localhost:7100"), None);
        assert_eq!(PublicUrl::from_relay_url("http://relay.example"), None);
    }

    #[test]
    fn a_summary_is_one_line_cut_with_an_ellipsis() {
        assert_eq!(summary("  a\n b\tc ", 10), "a b c");
        assert_eq!(summary("one two three four", 12), "one two…");
        assert_eq!(summary(&"x".repeat(50), 10), format!("{}…", "x".repeat(9)));
        assert!(
            summary(&"word ".repeat(100), DESCRIPTION_CHARS)
                .chars()
                .count()
                <= DESCRIPTION_CHARS
        );
    }

    fn content(tags: &str, name: &str) -> Option<String> {
        let at = tags.find(&format!("=\"{name}\" content=\""))?;
        let rest = &tags[at + name.len() + 13..];
        Some(rest[..rest.find('"')?].to_owned())
    }

    #[test]
    fn a_page_with_a_card_names_it_absolutely() {
        let apps = Apps::new([App::Social]).with_public_url(PublicUrl::parse("https://r.test"));
        let tags = Preview::new("ada / engine", "Computes <numbers> & \"more\"")
            .kind("profile")
            .at("/social/r/x/engine")
            .image("/social/og/r/x/engine.png", "ada / engine")
            .tags("ignored", App::Social, &apps);
        assert_eq!(content(&tags, "og:title").as_deref(), Some("ada / engine"));
        assert_eq!(content(&tags, "og:type").as_deref(), Some("profile"));
        assert_eq!(
            content(&tags, "og:description").as_deref(),
            Some("Computes &lt;numbers&gt; &amp; &quot;more&quot;")
        );
        assert_eq!(
            content(&tags, "og:url").as_deref(),
            Some("https://r.test/social/r/x/engine")
        );
        assert_eq!(
            content(&tags, "og:image").as_deref(),
            Some("https://r.test/social/og/r/x/engine.png")
        );
        assert_eq!(
            content(&tags, "twitter:image").as_deref(),
            Some("https://r.test/social/og/r/x/engine.png")
        );
        assert_eq!(content(&tags, "og:image:width").as_deref(), Some("1200"));
        assert_eq!(content(&tags, "og:image:height").as_deref(), Some("630"));
        assert_eq!(
            content(&tags, "twitter:card").as_deref(),
            Some("summary_large_image")
        );
    }

    #[test]
    fn a_page_without_a_card_gets_the_site_card_and_its_own_title() {
        let apps = Apps::new([App::Repo]).with_public_url(PublicUrl::parse("https://r.test"));
        let tags = Preview::default().tags("Issues", App::Repo, &apps);
        assert_eq!(content(&tags, "og:title").as_deref(), Some("Issues"));
        assert_eq!(
            content(&tags, "og:image").as_deref(),
            Some("https://r.test/repo/og/site.png")
        );
        assert_eq!(content(&tags, "og:url"), None);
        assert_eq!(content(&tags, "og:type").as_deref(), Some("website"));
    }

    #[test]
    fn without_a_public_url_no_image_or_url_is_named() {
        let tags = Preview::new("t", "d")
            .at("/social")
            .image("/social/og/site.png", "a")
            .tags("t", App::Social, &Apps::new([App::Social]));
        assert_eq!(content(&tags, "og:title").as_deref(), Some("t"));
        for absent in ["og:url", "og:image", "twitter:image"] {
            assert_eq!(content(&tags, absent), None, "{absent}");
        }
        assert_eq!(content(&tags, "twitter:card").as_deref(), Some("summary"));
    }
}
