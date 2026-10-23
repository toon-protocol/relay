//! Media a post names: its NIP-92 `imeta` tags (and the NIP-68/71 picture
//! and video events, which use the same tag), drawn as `<img>` and `<video>`
//! elements the visitor's browser loads.
//!
//! The relay never fetches media (ADR 0001). The `/social` pages allow
//! `img-src` and `media-src` of `https:` so the browser can, send
//! `Referrer-Policy: no-referrer` so the media host is not told which page
//! asked, and only an `https` URL is ever drawn: no `http:`, no `data:`.
//! A video is not fetched until it is played (`preload="none"`).

use nostr::event::Event;
use relay_ui::chrome::escape;

/// What a media URL holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum MediaKind {
    Image,
    Video,
}

/// One piece of media a post names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Media {
    url: String,
    kind: MediaKind,
    alt: Option<String>,
    poster: Option<String>,
}

impl Media {
    /// The media at `url`, if it is `https` and its type (from `mime`, else
    /// the URL's extension) is an image or a video.
    pub(crate) fn new(url: &str, mime: Option<&str>) -> Option<Self> {
        let url = https(url)?;
        let kind = match mime.map(str::to_ascii_lowercase) {
            Some(mime) if mime.starts_with("image/") => MediaKind::Image,
            Some(mime) if mime.starts_with("video/") => MediaKind::Video,
            Some(_) => return None,
            None => by_extension(url)?,
        };
        Some(Self {
            url: url.to_owned(),
            kind,
            alt: None,
            poster: None,
        })
    }

    /// The URL.
    pub(crate) fn url(&self) -> &str {
        &self.url
    }

    /// Whether it is a video.
    pub(crate) fn is_video(&self) -> bool {
        self.kind == MediaKind::Video
    }

    /// The element, with `class`.
    pub(crate) fn html(&self, class: &str) -> String {
        let alt = escape(self.alt.as_deref().unwrap_or(""));
        match self.kind {
            MediaKind::Image => format!(
                "<img class=\"{class}\" src=\"{}\" alt=\"{alt}\" loading=\"lazy\" decoding=\"async\" \
                 referrerpolicy=\"no-referrer\">",
                escape(&self.url)
            ),
            MediaKind::Video => format!(
                "<video class=\"{class}\" src=\"{}\" controls preload=\"none\" playsinline{}{}></video>",
                escape(&self.url),
                self.poster
                    .as_deref()
                    .map(|poster| format!(" poster=\"{}\"", escape(poster)))
                    .unwrap_or_default(),
                if alt.is_empty() {
                    String::new()
                } else {
                    format!(" aria-label=\"{alt}\"")
                },
            ),
        }
    }
}

/// `url` if it is `https` with a host and holds nothing a URL should not.
pub(crate) fn https(url: &str) -> Option<&str> {
    let url = url.trim();
    let rest = url.strip_prefix("https://")?;
    (!rest.is_empty()
        && !rest.starts_with('/')
        && !url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '"' || c == '<' || c == '>'))
    .then_some(url)
}

/// The kind of media a URL's extension says it is.
fn by_extension(url: &str) -> Option<MediaKind> {
    let path = url.split(['?', '#']).next().unwrap_or(url);
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    match extension.as_str() {
        "png" | "jpg" | "jpeg" | "gif" | "webp" | "avif" => Some(MediaKind::Image),
        "mp4" | "webm" | "mov" | "m4v" => Some(MediaKind::Video),
        _ => None,
    }
}

/// The media of `event`'s `imeta` tags, in order, each URL once. A pure
/// function.
pub(crate) fn imeta(event: &Event) -> Vec<Media> {
    let mut found: Vec<Media> = Vec::new();
    for tag in event.tags.iter().map(|tag| tag.as_slice()) {
        let Some((name, fields)) = tag.split_first() else {
            continue;
        };
        if name != "imeta" {
            continue;
        }
        let field = |key: &str| {
            fields.iter().find_map(|field| {
                field
                    .split_once(' ')
                    .filter(|(name, _)| *name == key)
                    .map(|(_, value)| value.trim())
            })
        };
        let Some(mut media) = field("url").and_then(|url| Media::new(url, field("m"))) else {
            continue;
        };
        media.alt = field("alt").map(str::to_owned);
        media.poster = field("image").and_then(https).map(str::to_owned);
        if !found.iter().any(|held| held.url == media.url) {
            found.push(media);
        }
    }
    found
}

/// The image an article or app names in its `image` tag, if it is `https`.
pub(crate) fn image_tag(event: &Event) -> Option<Media> {
    let url = ui_nip34::repo::first_tag(event, "image")?;
    let mut media = Media::new(url, None).or_else(|| Media::new(url, Some("image/*")))?;
    media.kind = MediaKind::Image;
    Some(media)
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Kind, Tag};
    use nostr::key::Keys;

    use super::*;

    fn note(tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::TextNote, "")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .finalize(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn only_https_images_and_videos_are_media() {
        let event = note(&[
            &[
                "imeta",
                "url https://cdn.test/a.png",
                "m image/png",
                "alt a chart",
            ],
            &[
                "imeta",
                "url https://cdn.test/demo",
                "m video/mp4",
                "image https://cdn.test/p.jpg",
            ],
            &["imeta", "url http://cdn.test/b.png", "m image/png"],
            &["imeta", "url data:image/png;base64,AAAA"],
            &["imeta", "url https://cdn.test/c.pdf", "m application/pdf"],
            &["imeta", "url https://cdn.test/d.webp"],
            &["imeta", "url https://cdn.test/a.png"],
            &["imeta", "url https://cdn.test/\"onerror=x.png"],
        ]);
        let media = imeta(&event);
        let urls: Vec<&str> = media.iter().map(Media::url).collect();
        assert_eq!(
            urls,
            [
                "https://cdn.test/a.png",
                "https://cdn.test/demo",
                "https://cdn.test/d.webp"
            ]
        );
        assert!(media[1].is_video());
        assert!(media[0].html("hero").contains("alt=\"a chart\""));
        assert!(media[1].html("hero").contains("preload=\"none\""));
        assert!(
            media[1]
                .html("hero")
                .contains("poster=\"https://cdn.test/p.jpg\"")
        );
    }
}
