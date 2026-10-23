//! Text on a card: cleaned, then wrapped into lines that fit a width, with
//! an ellipsis where it is cut. resvg draws one line per `<text>` and wraps
//! nothing itself, so the lines are decided here, against glyph widths a
//! [`Measure`] gives. Every function is pure; the tests measure with a fake
//! that gives each character the same width.

/// The most characters of any one text a card reads. A description may be
/// any length; a card shows a few lines of it, so the rest is never measured.
pub const MAX_CHARS: usize = 400;

/// How heavy a run of text is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Weight {
    Regular,
    Bold,
}

/// How wide text is drawn, and which characters the font can draw.
pub trait Measure {
    /// The advance width of `text` at `size` pixels, in pixels.
    fn width(&self, text: &str, size: f32, weight: Weight) -> f32;

    /// Whether the font has a glyph for `character`.
    fn supports(&self, character: char) -> bool;
}

/// `text` as a card draws it: white space collapsed to single spaces,
/// control characters and characters the font cannot draw left out, at most
/// [`MAX_CHARS`] characters.
pub fn clean(text: &str, measure: &impl Measure) -> String {
    let mut out = String::new();
    let mut space = false;
    for character in text.chars() {
        if character.is_whitespace() {
            space = !out.is_empty();
            continue;
        }
        if character.is_control() || !measure.supports(character) {
            continue;
        }
        if out.chars().count() >= MAX_CHARS {
            break;
        }
        if space {
            out.push(' ');
            space = false;
        }
        out.push(character);
    }
    out
}

/// One line of a [`wrap`]: the text and how wide it is drawn.
#[derive(Debug, Clone, PartialEq)]
pub struct Line {
    pub text: String,
    pub width: f32,
}

/// What [`wrap`] fits text to.
#[derive(Debug, Clone, Copy)]
pub struct Fit {
    pub size: f32,
    pub weight: Weight,
    /// The widest a line may be, in pixels.
    pub width: f32,
    /// The most lines; at least one.
    pub lines: usize,
}

/// `text` (already [`clean`]) broken into at most `fit.lines` lines no
/// wider than `fit.width`: at spaces where it can, inside a word too long
/// for a line otherwise. Text that does not fit ends its last line with `…`.
/// Empty text is no lines.
pub fn wrap(text: &str, fit: Fit, measure: &impl Measure) -> Vec<Line> {
    let width = |line: &str| measure.width(line, fit.size, fit.weight);
    let max_lines = fit.lines.max(1);
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut cut = false;
    'words: for word in text.split(' ').filter(|word| !word.is_empty()) {
        let joined = if current.is_empty() {
            word.to_owned()
        } else {
            format!("{current} {word}")
        };
        if width(&joined) <= fit.width {
            current = joined;
            continue;
        }
        if !current.is_empty() {
            lines.push(std::mem::take(&mut current));
            if lines.len() == max_lines {
                cut = true;
                break 'words;
            }
        }
        // The word alone: whole if it fits, else as many pieces as it takes.
        for character in word.chars() {
            let longer = format!("{current}{character}");
            if width(&longer) <= fit.width || current.is_empty() {
                current = longer;
            } else {
                lines.push(std::mem::replace(&mut current, character.to_string()));
                if lines.len() == max_lines {
                    cut = true;
                    break 'words;
                }
            }
        }
    }
    if !cut && !current.is_empty() {
        lines.push(current);
    }
    if cut && let Some(last) = lines.last_mut() {
        *last = ellipsis(last, fit, measure);
    }
    lines
        .into_iter()
        .map(|text| Line {
            width: width(&text),
            text,
        })
        .collect()
}

/// `line` shortened until it fits `fit.width` with `…` after it.
fn ellipsis(line: &str, fit: Fit, measure: &impl Measure) -> String {
    let mut kept: Vec<char> = line.chars().collect();
    loop {
        let shown: String = kept.iter().collect();
        let shown = format!("{}…", shown.trim_end());
        if kept.is_empty() || measure.width(&shown, fit.size, fit.weight) <= fit.width {
            return shown;
        }
        kept.pop();
    }
}

/// `text` (already [`clean`]) as one line that fits: [`wrap`] to one line.
pub fn line(text: &str, size: f32, weight: Weight, width: f32, measure: &impl Measure) -> Line {
    wrap(
        text,
        Fit {
            size,
            weight,
            width,
            lines: 1,
        },
        measure,
    )
    .pop()
    .unwrap_or(Line {
        text: String::new(),
        width: 0.0,
    })
}

/// The first two letters or digits of `name`, upper case: what an avatar
/// shows, as the pages draw it. Never a picture.
pub fn initials(name: &str) -> String {
    name.chars()
        .filter(|character| character.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_uppercase()
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// Every character is `size / 2` wide; `bold` ones a tenth wider; `☃`
    /// has no glyph.
    pub(crate) struct Fixed;

    impl Measure for Fixed {
        fn width(&self, text: &str, size: f32, weight: Weight) -> f32 {
            let each = match weight {
                Weight::Regular => size / 2.0,
                Weight::Bold => size * 0.55,
            };
            text.chars().count() as f32 * each
        }

        fn supports(&self, character: char) -> bool {
            character != '☃'
        }
    }

    fn fit(width: f32, lines: usize) -> Fit {
        // 10px wide characters.
        Fit {
            size: 20.0,
            weight: Weight::Regular,
            width,
            lines,
        }
    }

    fn texts(lines: &[Line]) -> Vec<&str> {
        lines.iter().map(|line| line.text.as_str()).collect()
    }

    #[test]
    fn cleaning_collapses_space_and_drops_what_cannot_be_drawn() {
        assert_eq!(clean("  a\n\tb  ☃c\u{7}d ", &Fixed), "a b cd");
        assert_eq!(clean("", &Fixed), "");
        assert_eq!(clean(&"x".repeat(5_000), &Fixed).len(), MAX_CHARS);
    }

    #[test]
    fn words_wrap_at_spaces_within_the_width() {
        let lines = wrap("the quick brown fox", fit(100.0, 3), &Fixed);
        assert_eq!(texts(&lines), ["the quick", "brown fox"]);
        assert!(lines.iter().all(|line| line.width <= 100.0));
        assert_eq!(lines[0].width, 90.0);
    }

    #[test]
    fn text_past_the_last_line_ends_in_an_ellipsis_that_fits() {
        let lines = wrap("one two three four five six", fit(90.0, 2), &Fixed);
        assert_eq!(texts(&lines), ["one two", "three…"]);
        let one = wrap("abcdefghijkl", fit(50.0, 1), &Fixed);
        assert_eq!(texts(&one), ["abcd…"]);
        assert!(one[0].width <= 50.0);
    }

    #[test]
    fn a_word_too_long_for_a_line_is_broken_inside() {
        let lines = wrap("abcdefghij xy", fit(40.0, 5), &Fixed);
        assert_eq!(texts(&lines), ["abcd", "efgh", "ij", "xy"]);
    }

    #[test]
    fn text_that_fits_is_left_whole_and_nothing_is_no_lines() {
        assert_eq!(texts(&wrap("short", fit(500.0, 1), &Fixed)), ["short"]);
        assert!(wrap("", fit(500.0, 3), &Fixed).is_empty());
        assert_eq!(line("", 20.0, Weight::Bold, 10.0, &Fixed).text, "");
    }

    #[test]
    fn a_line_never_comes_back_wider_than_it_may_be_unless_one_glyph_is() {
        for width in [5.0, 10.0, 25.0, 77.0, 130.0] {
            for lines in 1..4 {
                for line in wrap(
                    "lorem ipsum dolor sit amet consectetur",
                    fit(width, lines),
                    &Fixed,
                ) {
                    assert!(line.width <= width.max(20.0), "{width} {lines}: {line:?}");
                }
            }
        }
    }

    #[test]
    fn initials_are_the_first_two_letters_upper_case() {
        assert_eq!(initials("ada lovelace"), "AD");
        assert_eq!(initials("<b>"), "B");
        assert_eq!(initials("🙂"), "");
    }
}
