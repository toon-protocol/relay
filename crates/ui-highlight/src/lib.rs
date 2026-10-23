//! Highlighting as plain data: text in, classed runs out. No DOM and no
//! `wasm-bindgen`, so `cargo test` covers it natively and the relay can link
//! it. The colours are not here: a run carries a [`Class`], and the one
//! stylesheet (`relay-ui`) says what each class looks like, for `/repo`'s
//! WebAssembly module and for the server-rendered `/social` pages alike.

use std::str::FromStr;
use std::sync::OnceLock;

use syntect::highlighting::{
    Color, HighlightIterator, HighlightState, Highlighter, ScopeSelectors, Style, StyleModifier,
    Theme, ThemeItem, ThemeSettings,
};
use syntect::parsing::{ParseState, ScopeStack, ScopeStackOp, SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;

/// A file bigger than this is shown as plain text: the page must not stall on
/// a minified bundle.
pub const MAX_BYTES: usize = 512 * 1024;

/// The compiled set of the languages the page highlights (`build.rs`).
static SYNTAXES: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/syntaxes.packdump"));

/// What a run of text is, which the stylesheet turns into a colour.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Comment,
    String,
    Number,
    Escape,
    Keyword,
    Entity,
    Parameter,
    Heading,
    Bold,
    Italic,
    Raw,
}

impl Class {
    /// Every class the highlighter can emit.
    pub const ALL: [Class; 11] = [
        Class::Comment,
        Class::String,
        Class::Number,
        Class::Escape,
        Class::Keyword,
        Class::Entity,
        Class::Parameter,
        Class::Heading,
        Class::Bold,
        Class::Italic,
        Class::Raw,
    ];

    /// The CSS class the stylesheet styles.
    pub const fn css(self) -> &'static str {
        match self {
            Class::Comment => "hl-comment",
            Class::String => "hl-string",
            Class::Number => "hl-number",
            Class::Escape => "hl-escape",
            Class::Keyword => "hl-keyword",
            Class::Entity => "hl-entity",
            Class::Parameter => "hl-parameter",
            Class::Heading => "hl-heading",
            Class::Bold => "hl-bold",
            Class::Italic => "hl-italic",
            Class::Raw => "hl-raw",
        }
    }

    /// The marker colour the theme gives this class, so a style read back
    /// from syntect names it. Never zero: that is the unscoped text.
    fn marker(self) -> u32 {
        Self::ALL.iter().position(|&c| c == self).unwrap_or(0) as u32 + 1
    }

    fn from_marker(marker: u32) -> Option<Class> {
        Self::ALL.get((marker as usize).checked_sub(1)?).copied()
    }
}

/// One run of text in one class.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    /// The text, exactly as it is in the file.
    pub text: String,
    /// None for the page's own colour.
    pub class: Option<Class>,
}

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(|| syntect::dumps::from_binary(SYNTAXES))
}

fn item(scope: &str, class: Class) -> ThemeItem {
    let marker = class.marker();
    ThemeItem {
        scope: ScopeSelectors::from_str(scope).expect("the theme's selectors are fixed and valid"),
        style: StyleModifier {
            foreground: Some(Color {
                r: (marker >> 16) as u8,
                g: (marker >> 8) as u8,
                b: marker as u8,
                a: 0xff,
            }),
            background: None,
            font_style: None,
        },
    }
}

/// Which scopes are which class. The colours are the stylesheet's.
fn theme() -> Theme {
    Theme {
        settings: ThemeSettings::default(),
        scopes: vec![
            item("comment", Class::Comment),
            item("string", Class::String),
            item("constant.numeric, constant.language", Class::Number),
            item("constant.character.escape", Class::Escape),
            item("keyword, storage", Class::Keyword),
            item("entity.name, support.function, support.type", Class::Entity),
            item(
                "variable.parameter, variable.other.member, entity.other.attribute-name",
                Class::Parameter,
            ),
            item("markup.heading", Class::Heading),
            item("markup.bold", Class::Bold),
            item("markup.italic", Class::Italic),
            item("markup.raw, markup.underline.link", Class::Raw),
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
    run(syntaxes().find_syntax_by_extension(extension)?, text)
}

/// A snippet's `text` as runs. The language is its `l` tag, matched to a
/// language name or extension without regard to case, else its `extension`
/// tag with any leading `.` removed. None as for [`highlight`].
pub fn highlight_snippet(
    language: Option<&str>,
    extension: Option<&str>,
    text: &str,
) -> Option<Vec<Run>> {
    let set = syntaxes();
    let by_language = language
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .and_then(|token| set.find_syntax_by_token(token));
    let syntax = by_language.or_else(|| {
        let extension = extension?.trim().trim_start_matches('.');
        set.find_syntax_by_extension(extension)
    })?;
    run(syntax, text)
}

fn run(syntax: &SyntaxReference, text: &str) -> Option<Vec<Run>> {
    if text.len() > MAX_BYTES {
        return None;
    }
    let set = syntaxes();
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

/// Adds `piece`, merging it into the last run when the class is the same.
fn push(runs: &mut Vec<Run>, style: Style, piece: &str) {
    if piece.is_empty() {
        return;
    }
    let marker = u32::from(style.foreground.r) << 16
        | u32::from(style.foreground.g) << 8
        | u32::from(style.foreground.b);
    // The theme names no default colour, so unscoped text comes back black.
    let class = Class::from_marker(marker);
    match runs.last_mut() {
        Some(last) if last.class == class => last.text.push_str(piece),
        _ => runs.push(Run {
            text: piece.to_owned(),
            class,
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
        assert!(runs.iter().any(|run| run.class.is_some()));
        let keyword = runs
            .iter()
            .find(|run| run.text == "fn")
            .expect("fn is a run");
        let string = runs.iter().find(|run| run.text.contains("\"x\"")).unwrap();
        assert_eq!(keyword.class, Some(Class::Keyword));
        assert_eq!(string.class, Some(Class::String));
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
                runs.iter().any(|run| run.class.is_some()),
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
    fn a_snippets_language_tag_or_extension_picks_the_syntax() {
        let source = "const x: number = 1; // c\n";
        for language in ["TypeScript", "typescript", "ts", " ts "] {
            let runs = highlight_snippet(Some(language), None, source).expect(language);
            assert!(runs.iter().any(|run| run.class == Some(Class::Keyword)));
        }
        let rust = highlight_snippet(Some("rust"), None, "fn a() {}\n").expect("rust");
        assert_eq!(rust[0].class, Some(Class::Keyword));
        for extension in ["ts", ".ts"] {
            assert!(highlight_snippet(None, Some(extension), source).is_some());
        }
        // The language wins; an unknown one falls back to the extension.
        assert!(highlight_snippet(Some("klingon"), Some("rs"), "fn a() {}").is_some());
        assert!(highlight_snippet(Some("klingon"), None, source).is_none());
        assert!(highlight_snippet(None, None, source).is_none());
        assert!(highlight_snippet(Some(""), Some("."), source).is_none());
    }

    #[test]
    fn an_oversized_snippet_is_left_plain() {
        let big = "a".repeat(MAX_BYTES + 1);
        assert!(highlight_snippet(Some("rust"), None, &big).is_none());
    }

    #[test]
    fn every_class_has_a_distinct_marker_and_css_name() {
        for (i, class) in Class::ALL.into_iter().enumerate() {
            assert_eq!(Class::from_marker(class.marker()), Some(class));
            assert_eq!(class.marker() as usize, i + 1);
            assert!(class.css().starts_with("hl-"));
        }
        assert_eq!(Class::from_marker(0), None);
    }

    #[test]
    fn the_extension_is_the_last_dot_of_the_name() {
        assert_eq!(extension("main.rs"), Some("rs"));
        assert_eq!(extension("a.test.ts"), Some("ts"));
        assert_eq!(extension("Makefile"), None);
        assert_eq!(extension("trailing."), None);
    }
}
