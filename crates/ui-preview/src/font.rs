//! The one font a card is drawn in, compiled into the binary: Inter 4.1,
//! regular and bold, under the SIL Open Font License 1.1 (`fonts/OFL.txt`,
//! from <https://github.com/rsms/inter>). It is never read from the system,
//! so a card looks the same on every relay. It draws Latin, Greek and
//! Cyrillic; a character it has no glyph for is left out of a card
//! ([`crate::text::clean`]).
//!
//! The same bytes give [`Fonts`]' widths, which lay a card's text out, and
//! resvg's font database, which draws it.

use std::sync::{Arc, OnceLock};

use resvg::tiny_skia::{Pixmap, Transform};
use resvg::usvg::{self, fontdb};
use skrifa::instance::{LocationRef, Size};
use skrifa::{FontRef, MetadataProvider};

use crate::text::{Measure, Weight};
use crate::{HEIGHT, PreviewError, WIDTH};

/// The family name a card's SVG asks for.
pub const FAMILY: &str = "Inter";

const REGULAR: &[u8] = include_bytes!("../fonts/Inter-Regular.ttf");
const BOLD: &[u8] = include_bytes!("../fonts/Inter-Bold.ttf");

/// The bundled font's glyph widths.
pub struct Fonts {
    regular: FontRef<'static>,
    bold: FontRef<'static>,
}

impl Fonts {
    fn face(&self, weight: Weight) -> &FontRef<'static> {
        match weight {
            Weight::Regular => &self.regular,
            Weight::Bold => &self.bold,
        }
    }
}

impl Measure for Fonts {
    fn width(&self, text: &str, size: f32, weight: Weight) -> f32 {
        let face = self.face(weight);
        let charmap = face.charmap();
        let metrics = face.glyph_metrics(Size::new(size), LocationRef::default());
        text.chars()
            .filter_map(|character| charmap.map(character))
            .filter_map(|glyph| metrics.advance_width(glyph))
            .sum()
    }

    fn supports(&self, character: char) -> bool {
        self.regular.charmap().map(character).is_some()
            && self.bold.charmap().map(character).is_some()
    }
}

/// The bundled font, read once.
pub fn fonts() -> &'static Fonts {
    static FONTS: OnceLock<Fonts> = OnceLock::new();
    FONTS.get_or_init(|| Fonts {
        regular: FontRef::new(REGULAR).expect("the bundled Inter Regular is a valid font"),
        bold: FontRef::new(BOLD).expect("the bundled Inter Bold is a valid font"),
    })
}

/// resvg's font database: the bundled font and nothing else.
fn database() -> Arc<fontdb::Database> {
    static DATABASE: OnceLock<Arc<fontdb::Database>> = OnceLock::new();
    DATABASE
        .get_or_init(|| {
            let mut database = fontdb::Database::new();
            database.load_font_data(REGULAR.to_vec());
            database.load_font_data(BOLD.to_vec());
            database.set_sans_serif_family(FAMILY);
            Arc::new(database)
        })
        .clone()
}

/// `svg` drawn as a PNG of the card's size.
pub(crate) fn rasterise(svg: &str) -> Result<Vec<u8>, PreviewError> {
    let options = usvg::Options {
        font_family: FAMILY.to_owned(),
        fontdb: database(),
        ..usvg::Options::default()
    };
    let tree = usvg::Tree::from_str(svg, &options)
        .map_err(|error| PreviewError::Svg(error.to_string()))?;
    let mut pixmap = Pixmap::new(WIDTH, HEIGHT).ok_or(PreviewError::Pixmap)?;
    resvg::render(&tree, Transform::identity(), &mut pixmap.as_mut());
    pixmap
        .encode_png()
        .map_err(|error| PreviewError::Png(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_font_measures_bold_wider_and_knows_its_glyphs() {
        let fonts = fonts();
        let regular = fonts.width("engine", 40.0, Weight::Regular);
        let bold = fonts.width("engine", 40.0, Weight::Bold);
        assert!(regular > 50.0 && regular < 200.0, "{regular}");
        assert!(bold > regular);
        assert!((fonts.width("engine", 80.0, Weight::Regular) - 2.0 * regular).abs() < 0.01);
        assert!(fonts.supports('a') && fonts.supports('Ж') && fonts.supports('…'));
        assert!(!fonts.supports('漢'));
    }
}
