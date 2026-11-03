//! A picture a card draws: raster bytes whose format and size were read from
//! the bytes themselves, within bounds, before anything decodes them.

use std::sync::Arc;

use base64::Engine;
use sha2::{Digest, Sha256};

/// The widest or tallest a picture may be.
pub const MAX_SIDE: usize = 4096;

/// The most pixels a picture may hold (4096 × 4096), so decoding one for a
/// card costs at most 64 MiB.
pub const MAX_PIXELS: usize = MAX_SIDE * MAX_SIDE;

/// The raster formats a card draws. SVG is not one: a picture is pixels,
/// never a document of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Format {
    Png,
    Jpeg,
    Gif,
    Webp,
}

impl Format {
    /// The format `bytes` start as, by their magic number alone.
    fn of(bytes: &[u8]) -> Option<Self> {
        match bytes {
            [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, ..] => Some(Self::Png),
            [0xff, 0xd8, 0xff, ..] => Some(Self::Jpeg),
            [b'G', b'I', b'F', b'8', ..] => Some(Self::Gif),
            [
                b'R',
                b'I',
                b'F',
                b'F',
                _,
                _,
                _,
                _,
                b'W',
                b'E',
                b'B',
                b'P',
                ..,
            ] => Some(Self::Webp),
            _ => None,
        }
    }

    fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
            Self::Gif => "image/gif",
            Self::Webp => "image/webp",
        }
    }
}

/// A PNG, JPEG, GIF or WebP no bigger than [`MAX_SIDE`] a side and
/// [`MAX_PIXELS`] in all. Two are equal when their bytes are, and its
/// `Debug` names its digest, not its bytes, so a card's key stays short.
#[derive(Clone)]
pub struct Picture {
    format: Format,
    width: usize,
    height: usize,
    bytes: Arc<[u8]>,
    digest: [u8; 32],
}

impl Picture {
    /// `bytes` as a picture, or none when they are not one of the formats or
    /// are out of bounds. Only the header is read.
    pub fn new(bytes: Vec<u8>) -> Option<Self> {
        let format = Format::of(&bytes)?;
        let size = imagesize::blob_size(&bytes).ok()?;
        let (width, height) = (size.width, size.height);
        if width == 0
            || height == 0
            || width > MAX_SIDE
            || height > MAX_SIDE
            || width * height > MAX_PIXELS
        {
            return None;
        }
        let digest = Sha256::digest(&bytes).into();
        Some(Self {
            format,
            width,
            height,
            bytes: bytes.into(),
            digest,
        })
    }

    pub fn format(&self) -> Format {
        self.format
    }

    /// Width and height in pixels.
    pub fn size(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// How many bytes it holds.
    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bytes.is_empty()
    }

    /// The picture as a `data:` URL, for the card's SVG.
    pub(crate) fn data_url(&self) -> String {
        format!(
            "data:{};base64,{}",
            self.format.mime(),
            base64::engine::general_purpose::STANDARD.encode(&self.bytes)
        )
    }
}

impl std::fmt::Debug for Picture {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let digest: String = self.digest[..8]
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        write!(
            formatter,
            "Picture({:?} {}x{} {digest})",
            self.format, self.width, self.height
        )
    }
}

impl PartialEq for Picture {
    fn eq(&self, other: &Self) -> bool {
        self.digest == other.digest
    }
}

impl Eq for Picture {}

impl std::hash::Hash for Picture {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.digest.hash(state);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A PNG of `width` × `height`, one colour.
    pub(crate) fn png(width: u32, height: u32) -> Vec<u8> {
        coloured(width, height, (200, 80, 40))
    }

    /// A PNG of `width` × `height` in `(r, g, b)`.
    pub(crate) fn coloured(width: u32, height: u32, (r, g, b): (u8, u8, u8)) -> Vec<u8> {
        let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height).expect("a pixmap");
        pixmap.fill(resvg::tiny_skia::Color::from_rgba8(r, g, b, 255));
        pixmap.encode_png().expect("encodes")
    }

    #[test]
    fn a_picture_is_read_from_its_header() {
        let picture = Picture::new(png(40, 20)).expect("a picture");
        assert_eq!(picture.format(), Format::Png);
        assert_eq!(picture.size(), (40, 20));
        assert!(
            picture
                .data_url()
                .starts_with("data:image/png;base64,iVBOR")
        );
        assert!(format!("{picture:?}").starts_with("Picture(Png 40x20 "));
        assert_eq!(picture, Picture::new(png(40, 20)).expect("again"));
        assert_ne!(picture, Picture::new(png(41, 20)).expect("another"));
    }

    #[test]
    fn anything_else_is_not_a_picture() {
        let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>".to_vec();
        assert!(Picture::new(svg).is_none(), "an SVG is not a picture");
        assert!(Picture::new(Vec::new()).is_none());
        assert!(
            Picture::new(b"GIF8".to_vec()).is_none(),
            "a header and nothing after"
        );
        let mut huge = png(2, 2);
        // The IHDR's width, claimed far past the bound.
        huge[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        assert!(Picture::new(huge).is_none());
    }
}
