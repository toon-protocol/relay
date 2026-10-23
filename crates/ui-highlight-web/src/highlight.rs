//! Highlighting as plain data: text in, coloured runs out. No DOM, so
//! `cargo test` covers it natively.

use std::str::FromStr;
use std::sync::OnceLock;

use syntect::highlighting::{
    Color, FontStyle, HighlightIterator, HighlightState, Highlighter, Style, StyleModifier, Theme,
    ThemeItem, ThemeSettings,
};
use syntect::parsing::{ParseState, ScopeStack, ScopeStackOp, SyntaxSet};
use syntect::util::LinesWithEndings;

/// A file bigger than this is shown as plain text: the page must not stall on
/// a minified bundle.
pub const MAX_BYTES: usize = 512 * 1024;

/// The compiled set of the languages the page highlights (`build.rs`).
static SYNTAXES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/syntaxes.packdump"));

/// One run of text in one style.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// The text, exactly as it is in the file.
    pub text: String,
    /// `0xRRGGBB`, or none for the page's own colour.
    pub color: Option<u32>,
    /// Whether the run is drawn bold.
    pub bold: bool,
    /// Whether the run is drawn italic.
    pub italic: bool,
}

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(|| syntect::dumps::from_binary(SYNTAXES))
}

fn item(scope: &str, rgb: u32, font: Option<FontStyle>) -> ThemeItem {
    ThemeItem {
        scope: syntect::highlighting::ScopeSelectors::from_str(scope)
            .expect("the theme's selectors are fixed and valid"),
        style: StyleModifier {
            foreground: Some(Color {
                r: (rgb >> 16) as u8,
                g: (rgb >> 8) as u8,
                b: rgb as u8,
                a: 0xff,
            }),
            background: None,
            font_style: font,
        },
    }
}

/// Colours with enough contrast on both a light and a dark page.
fn theme() -> Theme {
    Theme {
        settings: ThemeSettings::default(),
        scopes: vec![
            item("comment", 0x6a8a6a, Some(FontStyle::ITALIC)),
            item("string", 0x2f8f6a, None),
            item("constant.numeric, constant.language", 0xc2703d, None),
            item("constant.character.escape", 0xb5499c, None),
            item("keyword, storage", 0x4d8ad8, None),
            item(
                "entity.name, support.function, support.type",
                0xb08a2e,
                None,
            ),
            item(
                "variable.parameter, variable.other.member, entity.other.attribute-name",
                0x8a7bd1,
                None,
            ),
            item("markup.heading", 0x4d8ad8, Some(FontStyle::BOLD)),
            item("markup.bold", 0xc2703d, Some(FontStyle::BOLD)),
            item("markup.italic", 0xc2703d, Some(FontStyle::ITALIC)),
            item("markup.raw, markup.underline.link", 0x2f8f6a, None),
        ],
        ..Theme::default()
    }
}

/// The file extension of a path's last segment, if it has one.
pub fn extension(name: &str) -> Option<&str> {
    name.rsplit_once('.')
        .map(|(_, extension)| extension)
        .filter(|extension| !extension.is_empty())
}

/// Whether the page can highlight files of this extension.
pub fn knows(extension: &str) -> bool {
    syntaxes().find_syntax_by_extension(extension).is_some()
}

/// `text` as runs, or none when the language is unknown, the text is too
/// big, or the highlighter gave up: the caller then leaves the plain text.
pub fn highlight(extension: &str, text: &str) -> Option<Vec<Run>> {
    if text.len() > MAX_BYTES {
        return None;
    }
    let set = syntaxes();
    let syntax = set.find_syntax_by_extension(extension)?;
    let theme = theme();
    let highlighter = Highlighter::new(&theme);
    let mut parse = ParseState::new(syntax);
    let mut state = HighlightState::new(&highlighter, ScopeStack::new());
    let mut runs: Vec<Run> = Vec::new();
    for line in LinesWithEndings::from(text) {
        let ops: Vec<(usize, ScopeStackOp)> = parse.parse_line(line, set).ok()?;
        for (style, piece) in HighlightIterator::new(&mut state, &ops, line, &highlighter) {
            push(&mut runs, style, piece);
        }
    }
    Some(runs)
}

/// Adds `piece`, merging it into the last run when the style is the same.
fn push(runs: &mut Vec<Run>, style: Style, piece: &str) {
    if piece.is_empty() {
        return;
    }
    let color = Some(
        u32::from(style.foreground.r) << 16
            | u32::from(style.foreground.g) << 8
            | u32::from(style.foreground.b),
    );
    let bold = style.font_style.contains(FontStyle::BOLD);
    let italic = style.font_style.contains(FontStyle::ITALIC);
    // The theme names no default colour, so unscoped text comes back black.
    let color = color.filter(|&rgb| rgb != 0);
    match runs.last_mut() {
        Some(last) if last.color == color && last.bold == bold && last.italic == italic => {
            last.text.push_str(piece);
        }
        _ => runs.push(Run {
            text: piece.to_owned(),
            color,
            bold,
            italic,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn joined(runs: &[Run]) -> String {
        runs.iter().map(|run| run.text.as_str()).collect()
    }

    #[test]
    fn rust_is_highlighted_and_loses_no_text() {
        let source = "// hi\nfn main() {\n    let s = \"x\";\n}\n";
        let runs = highlight("rs", source).expect("rs is known");
        assert_eq!(joined(&runs), source);
        assert!(runs.iter().any(|run| run.color.is_some()));
        let keyword = runs
            .iter()
            .find(|run| run.text == "fn")
            .expect("fn is a run");
        let string = runs.iter().find(|run| run.text.contains("\"x\"")).unwrap();
        assert_ne!(keyword.color, string.color);
        assert!(keyword.color.is_some());
    }

    #[test]
    fn every_language_the_page_promises_is_known() {
        for extension in [
            "rs", "ts", "js", "json", "toml", "yaml", "yml", "md", "sh", "py", "go", "sol", "html",
            "css",
        ] {
            assert!(knows(extension), "{extension}");
        }
    }

    #[test]
    fn every_language_colours_something_and_keeps_the_text() {
        let samples = [
            ("rs", "fn a() -> u8 { 1 }\n"),
            ("ts", "interface A { b: number }\nconst x: A = { b: 1 };\n"),
            ("js", "const x = () => 'a';\n"),
            ("json", "{\"a\": [1, true, \"b\"]}\n"),
            ("toml", "[package]\nname = \"x\"\n"),
            ("yaml", "a:\n  - b: 1\n"),
            ("md", "# Title\n\n*it* and `code`\n"),
            ("sh", "echo \"$HOME\" # c\n"),
            ("py", "def f(x):\n    return 'a'\n"),
            ("go", "func main() { x := 1 }\n"),
            (
                "sol",
                "// c\ncontract A { function f() public pure returns (uint256) { return 1; } }\n",
            ),
            ("html", "<p class=\"a\">hi</p>\n"),
            ("css", "a { color: red; }\n"),
        ];
        for (extension, source) in samples {
            let runs = highlight(extension, source).expect(extension);
            assert_eq!(joined(&runs), source, "{extension}");
            assert!(
                runs.iter().any(|run| run.color.is_some()),
                "{extension} has colour"
            );
        }
    }

    #[test]
    fn an_unknown_extension_or_a_huge_file_is_left_plain() {
        assert!(highlight("zzz", "anything").is_none());
        assert!(highlight("rs", &"a".repeat(MAX_BYTES + 1)).is_none());
    }

    #[test]
    fn the_extension_is_the_last_dot_of_the_name() {
        assert_eq!(extension("main.rs"), Some("rs"));
        assert_eq!(extension("a.test.ts"), Some("ts"));
        assert_eq!(extension("Makefile"), None);
        assert_eq!(extension("trailing."), None);
    }
}
