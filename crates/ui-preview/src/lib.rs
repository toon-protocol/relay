//! Link preview cards for the app UIs (#314, ADR 0006): the 1200×630 image a
//! page's `og:image` names, so a `/social` or `/repo` link posted on X,
//! Discord, Slack or Bluesky unfurls into a card.
//!
//! The relay draws each card itself, from what its own store says, and
//! fetches nothing to do it (ADR 0001): an app reads the events and fills in a
//! [`Card`], [`draw`] writes it as SVG of text and plain shapes in the
//! pages' light colours, and [`Card::png`] rasterises that with `resvg` and
//! the bundled Inter font ([`font`]). A card never holds a picture: an
//! avatar is initials, as on the pages (ADR 0005). [`Previews`] serves cards
//! from a bounded cache, keyed by what each one shows, and draws only a few
//! at once, so a crawler cannot make the relay draw without limit.

mod draw;
pub mod font;
mod serve;
pub mod text;

pub use draw::draw;
pub use serve::{CACHE_CONTROL_VALUE, Previews};

use sha2::{Digest, Sha256};

/// A card's width, in pixels: what Open Graph and X's large card expect.
pub const WIDTH: u32 = 1200;
/// A card's height, in pixels.
pub const HEIGHT: u32 = 630;

/// Changes when the drawing does, so a cached card from before is not
/// served as current.
const DRAWING: &str = "ui-preview/1";

/// Which app a card is from: the mark in its corner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mark {
    /// `/repo`.
    Repos,
    /// `/social`.
    Social,
}

impl Mark {
    /// What the mark says after "TOON".
    pub fn label(self) -> &'static str {
        match self {
            Self::Repos => "Repos",
            Self::Social => "Social",
        }
    }
}

/// A repository, as its page counts it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Repo {
    /// The owner's display name, else the start of their npub.
    pub owner: String,
    pub name: String,
    pub description: Option<String>,
    /// The announcement's `t` tags; a card shows the first few that fit.
    pub topics: Vec<String>,
    /// Distinct people.
    pub stars: usize,
    /// Distinct people.
    pub followers: usize,
    /// As the page counts forks; none where the page shows no count.
    pub forks: Option<usize>,
}

/// A builder's profile.
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Profile {
    pub name: String,
    /// The NIP-05 identifier the profile claims. It is a claim: the relay
    /// checks none, and the card never calls it verified.
    pub nip05: Option<String>,
    pub about: Option<String>,
    /// Repos the key owns or maintains.
    pub repos: usize,
    /// Distinct people whose follow list names the key.
    pub followers: usize,
    /// The profile says `bot: true` (NIP-24).
    pub bot: bool,
}

/// A long-form article (NIP-23).
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash)]
pub struct Article {
    pub title: String,
    pub author: String,
    pub summary: Option<String>,
}

/// What a card shows.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Face {
    Repo(Repo),
    /// A ship post: its headline over the repo it launches.
    Ship {
        headline: String,
        repo: Repo,
    },
    Profile(Profile),
    Article(Article),
    /// A page with no card of its own: the app's name and what it is.
    Site {
        title: String,
        tagline: String,
    },
}

/// A preview card: what it shows, which app it is from, and the host the
/// relay is reached at, for the footer.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Card {
    mark: Mark,
    host: Option<String>,
    face: Face,
}

impl Card {
    /// A card from `mark`'s app showing `face`, with `host` in its footer
    /// when it is known.
    pub fn new(mark: Mark, host: Option<String>, face: Face) -> Self {
        Self { mark, host, face }
    }

    /// The app's mark.
    pub fn mark(&self) -> Mark {
        self.mark
    }

    /// The footer's host.
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// What the card shows.
    pub fn face(&self) -> &Face {
        &self.face
    }

    /// A name for what the card shows: the same for two cards that draw the
    /// same picture, different for any change in what is drawn. Its ETag.
    pub fn key(&self) -> String {
        let digest = Sha256::digest(format!("{DRAWING}\n{self:?}").as_bytes());
        digest[..16]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }

    /// The card as SVG, drawn with the bundled font's widths.
    pub fn svg(&self) -> String {
        draw(self, font::fonts())
    }

    /// The card as a PNG of [`WIDTH`] × [`HEIGHT`].
    pub fn png(&self) -> Result<Vec<u8>, PreviewError> {
        font::rasterise(&self.svg())
    }
}

/// The host a card's footer names: the public origin's when the operator
/// set one, else the relay hint visitors' clients use, else none.
pub fn footer_host(apps: &relay_ui::chrome::Apps, relay_host: Option<&str>) -> Option<String> {
    apps.public_url()
        .map(|url| url.host().to_owned())
        .or_else(|| relay_host.map(str::to_owned))
}

/// The site card of `mark`'s app: its name and what it is.
pub fn site(mark: Mark, host: Option<String>) -> Card {
    let app = match mark {
        Mark::Repos => relay_ui::chrome::App::Repo,
        Mark::Social => relay_ui::chrome::App::Social,
    };
    Card::new(
        mark,
        host,
        Face::Site {
            title: format!("TOON {}", mark.label()),
            tagline: relay_ui::meta::tagline(app).to_owned(),
        },
    )
}

/// The part of a card's file name before `.png`: `<npub>.png` is `<npub>`.
/// None for a name that is not a PNG's.
pub fn stem(file: &str) -> Option<&str> {
    file.strip_suffix(".png").filter(|stem| !stem.is_empty())
}

/// Why a card could not be drawn. Each is a bug in the drawing, not in the
/// events: what a card shows is escaped text.
#[derive(Debug, thiserror::Error)]
pub enum PreviewError {
    #[error("the card's SVG did not parse: {0}")]
    Svg(String),
    #[error("the card could not be encoded as PNG: {0}")]
    Png(String),
    #[error("no pixmap of the card's size could be made")]
    Pixmap,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> Repo {
        Repo {
            owner: "ada".into(),
            name: "engine".into(),
            description: Some("Computes Bernoulli numbers.".into()),
            topics: vec!["math".into()],
            stars: 3,
            followers: 1,
            forks: Some(2),
        }
    }

    #[test]
    fn the_key_changes_with_what_is_drawn_and_only_with_it() {
        let card = Card::new(Mark::Social, None, Face::Repo(repo()));
        assert_eq!(card.key(), card.clone().key());
        assert_eq!(card.key().len(), 32);
        let starred = Card::new(Mark::Social, None, Face::Repo(Repo { stars: 4, ..repo() }));
        assert_ne!(card.key(), starred.key());
        let hosted = Card::new(Mark::Social, Some("relay.test".into()), Face::Repo(repo()));
        assert_ne!(card.key(), hosted.key());
        assert_ne!(
            card.key(),
            Card::new(Mark::Repos, None, Face::Repo(repo())).key()
        );
    }

    /// The width and height a PNG's header states, if it is a PNG.
    pub(crate) fn png_size(bytes: &[u8]) -> Option<(u32, u32)> {
        let signature = b"\x89PNG\r\n\x1a\n";
        if bytes.len() < 24 || &bytes[..8] != signature || &bytes[12..16] != b"IHDR" {
            return None;
        }
        let word = |at: usize| {
            u32::from_be_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
        };
        Some((word(16), word(20)))
    }

    #[test]
    fn a_card_is_a_png_of_the_open_graph_size() {
        let card = Card::new(Mark::Social, Some("relay.test".into()), Face::Repo(repo()));
        let bytes = card.png().expect("a card draws");
        assert_eq!(png_size(&bytes), Some((WIDTH, HEIGHT)));
        assert!(bytes.len() < 512 * 1024, "{}", bytes.len());
    }

    #[tokio::test]
    async fn a_card_is_served_once_drawn_then_from_its_etag() {
        use axum::body::to_bytes;
        use axum::http::{HeaderMap, HeaderValue, StatusCode, header};

        let previews = Previews::new();
        let missing = previews.respond(&HeaderMap::new(), None).await;
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);

        let card = Card::new(Mark::Repos, None, Face::Repo(repo()));
        let first = previews
            .respond(&HeaderMap::new(), Some(card.clone()))
            .await;
        assert_eq!(first.status(), StatusCode::OK);
        assert_eq!(first.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(first.headers()[header::CACHE_CONTROL], CACHE_CONTROL_VALUE);
        let etag = first.headers()[header::ETAG].clone();
        assert_eq!(etag, format!("\"{}\"", card.key()).as_str());
        let bytes = to_bytes(first.into_body(), usize::MAX).await.unwrap();
        assert_eq!(png_size(&bytes), Some((WIDTH, HEIGHT)));
        assert_eq!(previews.held(), (1, bytes.len()));

        let mut asking = HeaderMap::new();
        asking.insert(header::IF_NONE_MATCH, etag.clone());
        let again = previews.respond(&asking, Some(card.clone())).await;
        assert_eq!(again.status(), StatusCode::NOT_MODIFIED);
        assert_eq!(again.headers()[header::ETAG], etag);

        let mut stale = HeaderMap::new();
        stale.insert(header::IF_NONE_MATCH, HeaderValue::from_static("\"old\""));
        let cached = previews.respond(&stale, Some(card)).await;
        assert_eq!(cached.status(), StatusCode::OK);
        assert_eq!(previews.held().0, 1);
    }
}
