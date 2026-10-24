//! A card drawn as SVG: GitHub's social card layout in the pages' light
//! colours (`relay-ui`'s `style.css` tokens). Text, rectangles, circles and
//! the pages' own Octicon paths, nothing else: no `<image>`, no link, no
//! font but the bundled one. Every string from an event is [`clean`]ed,
//! measured and escaped here, so the SVG's size and the time it takes to
//! draw are bounded whatever the events hold. A pure function of the card
//! and a [`Measure`].

use relay_ui::chrome::icon;

use crate::font::FAMILY;
use crate::text::{Fit, Line, Measure, Weight, clean, initials, line, wrap};
use crate::{Article, Card, Face, HEIGHT, Mark, Profile, Repo, WIDTH};

// The light theme's tokens.
const BG: &str = "#ffffff";
const FG: &str = "#1f2328";
const MUTED_FG: &str = "#656d76";
const BORDER: &str = "#d1d9e0";
const SUBTLE: &str = "#eaeef2";
const ACCENT_BG: &str = "#ddf4ff";
const ACCENT_FG: &str = "#0550ae";
const ATTENTION_BG: &str = "#fff8c5";
const ATTENTION_BORDER: &str = "#d4a72c";
const ATTENTION_FG: &str = "#7d4e00";
const LINK: &str = "#0969da";
const DONE: &str = "#8250df";
const SUCCESS: &str = "#1f883d";

/// An avatar's two colours, by name: the pages' accent, done, success and
/// attention pairs.
const AVATARS: [(&str, &str); 4] = [
    (ACCENT_BG, ACCENT_FG),
    ("#fbefff", DONE),
    ("#dafbe1", "#116329"),
    (ATTENTION_BG, ATTENTION_FG),
];

/// The left and right edges of what is drawn.
const LEFT: f32 = 80.0;
const RIGHT: f32 = WIDTH as f32 - 80.0;
/// The avatar, top right.
const AVATAR_X: f32 = 1040.0;
const AVATAR_Y: f32 = 214.0;
const AVATAR_R: f32 = 80.0;
/// Text beside the avatar stops this far left of it.
const BESIDE_AVATAR: f32 = AVATAR_X - AVATAR_R - 40.0 - LEFT;
/// The baseline of the footer: counts at the left, the host at the right.
const FOOTER: f32 = 568.0;
/// The lowest a line of the face's text may sit.
const LOWEST: f32 = 488.0;

/// `card` as an SVG document of [`WIDTH`] × [`HEIGHT`], its text laid out
/// with `measure`'s widths.
pub fn draw(card: &Card, measure: &impl Measure) -> String {
    let mut svg = Svg {
        out: String::new(),
        measure,
    };
    svg.out.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{WIDTH}\" height=\"{HEIGHT}\" \
         viewBox=\"0 0 {WIDTH} {HEIGHT}\"><defs><linearGradient id=\"band\" x1=\"0\" x2=\"1\" \
         y1=\"0\" y2=\"0\"><stop offset=\"0\" stop-color=\"{LINK}\"/><stop offset=\"0.5\" \
         stop-color=\"{DONE}\"/><stop offset=\"1\" stop-color=\"{SUCCESS}\"/></linearGradient>\
         </defs><rect width=\"{WIDTH}\" height=\"{HEIGHT}\" fill=\"{BG}\"/>"
    ));
    svg.mark(card.mark());
    let counts = match card.face() {
        Face::Repo(repo) => svg.repo(repo),
        Face::Ship { headline, repo } => svg.ship(headline, repo),
        Face::Profile(profile) => svg.profile(profile),
        Face::Article(article) => svg.article(article),
        Face::Site { title, tagline } => svg.site(title, tagline),
    };
    let used = svg.counts(&counts);
    if let Some(host) = card.host() {
        let host = clean(host, measure);
        let room = RIGHT - LEFT - used - 40.0;
        let shown = line(&host, 28.0, Weight::Regular, room, measure);
        if !shown.text.is_empty() {
            svg.text(
                RIGHT,
                FOOTER,
                (28.0, Weight::Regular),
                MUTED_FG,
                Anchor::End,
                &shown.text,
            );
        }
    }
    svg.out.push_str(&format!(
        "<rect y=\"{}\" width=\"{WIDTH}\" height=\"14\" fill=\"url(#band)\"/></svg>",
        HEIGHT - 14
    ));
    svg.out
}

/// What a count in the footer is: its icon, its number and what it counts.
struct Count {
    icon: &'static str,
    number: usize,
    singular: &'static str,
    plural: &'static str,
}

#[derive(Clone, Copy)]
enum Anchor {
    Start,
    Middle,
    End,
}

struct Svg<'a, M> {
    out: String,
    measure: &'a M,
}

impl<M: Measure> Svg<'_, M> {
    fn text(
        &mut self,
        x: f32,
        y: f32,
        (size, weight): (f32, Weight),
        fill: &str,
        anchor: Anchor,
        text: &str,
    ) {
        if text.is_empty() {
            return;
        }
        self.out.push_str(&format!(
            "<text x=\"{x:.1}\" y=\"{y:.1}\" font-family=\"{FAMILY}\" font-size=\"{size}\" \
             font-weight=\"{}\" fill=\"{fill}\"{}>{}</text>",
            match weight {
                Weight::Regular => 400,
                Weight::Bold => 700,
            },
            match anchor {
                Anchor::Start => "",
                Anchor::Middle => " text-anchor=\"middle\"",
                Anchor::End => " text-anchor=\"end\"",
            },
            escape(text),
        ));
    }

    /// An Octicon (a 16 unit path) `size` pixels square, its top left at
    /// `(x, y)`.
    fn icon(&mut self, path: &str, x: f32, y: f32, size: f32, fill: &str) {
        self.out.push_str(&format!(
            "<path transform=\"translate({x:.1} {y:.1}) scale({:.4})\" fill=\"{fill}\" d=\"{path}\"/>",
            size / 16.0
        ));
    }

    /// Lines of text from `y` (the first baseline) down, `height` apart.
    /// The baseline after the last.
    fn lines(&mut self, lines: &[Line], y: f32, height: f32, style: (f32, Weight, &str)) -> f32 {
        let (size, weight, fill) = style;
        let mut baseline = y;
        for line in lines {
            self.text(
                LEFT,
                baseline,
                (size, weight),
                fill,
                Anchor::Start,
                &line.text,
            );
            baseline += height;
        }
        baseline
    }

    /// The most lines `height` apart from `y` with none below [`LOWEST`].
    fn room(y: f32, height: f32, most: usize) -> usize {
        if y > LOWEST {
            return 0;
        }
        (((LOWEST - y) / height) as usize + 1).min(most)
    }

    /// `text` wrapped at `size` into at most `most` lines of `width` that
    /// fit above [`LOWEST`] from `y`.
    fn wrapped(
        &self,
        text: &str,
        (size, weight): (f32, Weight),
        width: f32,
        y: f32,
        height: f32,
        most: usize,
    ) -> Vec<Line> {
        let lines = Self::room(y, height, most);
        if lines == 0 {
            return Vec::new();
        }
        wrap(
            &clean(text, self.measure),
            Fit {
                size,
                weight,
                width,
                lines,
            },
            self.measure,
        )
    }

    /// The app's mark, top left: a dark tile with its icon, then
    /// `TOON <app>`.
    fn mark(&mut self, mark: Mark) {
        self.out.push_str(&format!(
            "<rect x=\"{LEFT}\" y=\"60\" width=\"52\" height=\"52\" rx=\"12\" fill=\"{FG}\"/>"
        ));
        let glyph = match mark {
            Mark::Repos => icon::REPO,
            Mark::Social => icon::COMMENT,
        };
        self.icon(glyph, LEFT + 12.0, 72.0, 28.0, BG);
        let x = LEFT + 72.0;
        self.text(x, 97.0, (30.0, Weight::Bold), FG, Anchor::Start, "TOON");
        let toon = self.measure.width("TOON", 30.0, Weight::Bold);
        self.text(
            x + toon + 10.0,
            97.0,
            (30.0, Weight::Regular),
            MUTED_FG,
            Anchor::Start,
            mark.label(),
        );
    }

    /// A round avatar with `name`'s initials, top right.
    fn avatar(&mut self, name: &str) {
        let shown = initials(&clean(name, self.measure));
        let pick = name
            .bytes()
            .fold(0usize, |sum, byte| sum + usize::from(byte))
            % AVATARS.len();
        let (fill, ink) = AVATARS[pick];
        self.out.push_str(&format!(
            "<circle cx=\"{AVATAR_X}\" cy=\"{AVATAR_Y}\" r=\"{AVATAR_R}\" fill=\"{fill}\" \
             stroke=\"{BORDER}\" stroke-width=\"2\"/>"
        ));
        self.text(
            AVATAR_X,
            AVATAR_Y + 22.0,
            (62.0, Weight::Bold),
            ink,
            Anchor::Middle,
            &shown,
        );
    }

    /// A pill: `label` (with `glyph` before it) at `(x, top)`, 44 high. Its
    /// width.
    fn chip(
        &mut self,
        x: f32,
        top: f32,
        glyph: Option<&str>,
        label: &str,
        colours: (&str, &str, &str),
    ) -> f32 {
        let (fill, border, ink) = colours;
        let size = 24.0;
        let text_width = self.measure.width(label, size, Weight::Bold);
        let glyph_width = if glyph.is_some() { 30.0 } else { 0.0 };
        let width = 40.0 + glyph_width + text_width;
        self.out.push_str(&format!(
            "<rect x=\"{x:.1}\" y=\"{top:.1}\" width=\"{width:.1}\" height=\"44\" rx=\"22\" \
             fill=\"{fill}\" stroke=\"{border}\" stroke-width=\"2\"/>"
        ));
        if let Some(glyph) = glyph {
            self.icon(glyph, x + 18.0, top + 11.0, 22.0, ink);
        }
        self.text(
            x + 20.0 + glyph_width,
            top + 30.5,
            (size, Weight::Bold),
            ink,
            Anchor::Start,
            label,
        );
        width
    }

    /// A title in bold at `size` with its baseline at `y`, on one line
    /// beside the avatar when it fits, else at `smaller` on two. The last
    /// line's baseline.
    fn title(&mut self, text: &str, y: f32, size: f32, smaller: f32, width: f32) -> f32 {
        let text = clean(text, self.measure);
        let whole = line(&text, size, Weight::Bold, width, self.measure);
        if whole.text == text {
            self.text(
                LEFT,
                y,
                (size, Weight::Bold),
                FG,
                Anchor::Start,
                &whole.text,
            );
            return y;
        }
        let lines = wrap(
            &text,
            Fit {
                size: smaller,
                weight: Weight::Bold,
                width,
                lines: 2,
            },
            self.measure,
        );
        let height = smaller * 1.15;
        let first = y - (size - smaller);
        self.lines(&lines, first, height, (smaller, Weight::Bold, FG)) - height
    }

    /// The face's topics as pills from `top`, as many as fit on one line,
    /// at most four, when there is room above the footer.
    fn topics(&mut self, topics: &[String], top: f32) {
        if top + 44.0 > FOOTER - 52.0 {
            return;
        }
        let mut x = LEFT;
        for topic in topics.iter().take(4) {
            let topic = line(
                &clean(topic, self.measure),
                24.0,
                Weight::Bold,
                300.0,
                self.measure,
            )
            .text;
            if topic.is_empty() {
                continue;
            }
            let width = 40.0 + self.measure.width(&topic, 24.0, Weight::Bold);
            if x + width > RIGHT {
                break;
            }
            x += self.chip(x, top, None, &topic, (ACCENT_BG, ACCENT_BG, ACCENT_FG)) + 12.0;
        }
    }

    /// The footer's counts, left to right. How wide they are.
    fn counts(&mut self, counts: &[Count]) -> f32 {
        let size = 30.0;
        let mut x = LEFT;
        for count in counts {
            self.icon(count.icon, x, FOOTER - 25.0, 28.0, MUTED_FG);
            x += 40.0;
            let number = compact(count.number);
            self.text(x, FOOTER, (size, Weight::Bold), FG, Anchor::Start, &number);
            x += self.measure.width(&number, size, Weight::Bold) + 9.0;
            let label = if count.number == 1 {
                count.singular
            } else {
                count.plural
            };
            self.text(
                x,
                FOOTER,
                (size, Weight::Regular),
                MUTED_FG,
                Anchor::Start,
                label,
            );
            x += self.measure.width(label, size, Weight::Regular) + 40.0;
        }
        (x - LEFT - 40.0).max(0.0)
    }

    fn repo(&mut self, repo: &Repo) -> Vec<Count> {
        self.avatar(&repo.owner);
        let owner = line(
            &format!("{} /", clean(&repo.owner, self.measure)),
            40.0,
            Weight::Regular,
            BESIDE_AVATAR,
            self.measure,
        );
        self.text(
            LEFT,
            196.0,
            (40.0, Weight::Regular),
            MUTED_FG,
            Anchor::Start,
            &owner.text,
        );
        let last = self.title(&repo.name, 284.0, 80.0, 60.0, BESIDE_AVATAR);
        let y = (last + 64.0).max(AVATAR_Y + AVATAR_R + 50.0);
        let description = repo
            .description
            .as_deref()
            .map(|text| self.wrapped(text, (32.0, Weight::Regular), RIGHT - LEFT, y, 44.0, 3))
            .unwrap_or_default();
        let next = self.lines(&description, y, 44.0, (32.0, Weight::Regular, MUTED_FG));
        let top = if description.is_empty() {
            y - 30.0
        } else {
            next - 18.0
        };
        self.topics(&repo.topics, top);
        repo_counts(repo)
    }

    fn ship(&mut self, headline: &str, repo: &Repo) -> Vec<Count> {
        self.chip(
            LEFT,
            148.0,
            Some(icon::SHIP),
            "Shipped",
            (ATTENTION_BG, ATTENTION_BORDER, ATTENTION_FG),
        );
        let headline = self.wrapped(headline, (60.0, Weight::Bold), RIGHT - LEFT, 262.0, 72.0, 2);
        let mut y = self.lines(&headline, 262.0, 72.0, (60.0, Weight::Bold, FG));
        if headline.is_empty() {
            y = 262.0;
        }
        y += 4.0;
        self.icon(icon::REPO, LEFT, y - 30.0, 34.0, MUTED_FG);
        let owner = line(
            &format!("{} /", clean(&repo.owner, self.measure)),
            38.0,
            Weight::Regular,
            (RIGHT - LEFT - 48.0) / 2.0,
            self.measure,
        );
        self.text(
            LEFT + 48.0,
            y,
            (38.0, Weight::Regular),
            MUTED_FG,
            Anchor::Start,
            &owner.text,
        );
        let gap = 12.0;
        let name = line(
            &clean(&repo.name, self.measure),
            38.0,
            Weight::Bold,
            RIGHT - LEFT - 48.0 - owner.width - gap,
            self.measure,
        );
        self.text(
            LEFT + 48.0 + owner.width + gap,
            y,
            (38.0, Weight::Bold),
            FG,
            Anchor::Start,
            &name.text,
        );
        let y = y + 54.0;
        let description = repo
            .description
            .as_deref()
            .map(|text| self.wrapped(text, (32.0, Weight::Regular), RIGHT - LEFT, y, 44.0, 2))
            .unwrap_or_default();
        self.lines(&description, y, 44.0, (32.0, Weight::Regular, MUTED_FG));
        repo_counts(repo)
    }

    fn profile(&mut self, profile: &Profile) -> Vec<Count> {
        self.avatar(&profile.name);
        let mut last = self.title(&profile.name, 240.0, 80.0, 60.0, BESIDE_AVATAR);
        if profile.nip05.is_some() || profile.bot {
            last += 58.0;
            let mut x = LEFT;
            if let Some(nip05) = &profile.nip05 {
                let nip05 = line(
                    &clean(nip05, self.measure),
                    34.0,
                    Weight::Regular,
                    BESIDE_AVATAR - 110.0,
                    self.measure,
                );
                self.text(
                    LEFT,
                    last,
                    (34.0, Weight::Regular),
                    MUTED_FG,
                    Anchor::Start,
                    &nip05.text,
                );
                x += nip05.width + 18.0;
            }
            if profile.bot {
                self.chip(x, last - 32.0, None, "bot", (SUBTLE, BORDER, MUTED_FG));
            }
        }
        let y = (last + 60.0).max(AVATAR_Y + AVATAR_R + 50.0);
        let about = profile
            .about
            .as_deref()
            .map(|text| self.wrapped(text, (34.0, Weight::Regular), RIGHT - LEFT, y, 46.0, 3))
            .unwrap_or_default();
        self.lines(&about, y, 46.0, (34.0, Weight::Regular, MUTED_FG));
        vec![
            Count {
                icon: icon::REPO,
                number: profile.repos,
                singular: "repository",
                plural: "repositories",
            },
            Count {
                icon: icon::PERSON,
                number: profile.followers,
                singular: "follower",
                plural: "followers",
            },
        ]
    }

    fn article(&mut self, article: &Article) -> Vec<Count> {
        self.chip(
            LEFT,
            148.0,
            Some(icon::BOOK),
            "Article",
            (ACCENT_BG, ACCENT_BG, ACCENT_FG),
        );
        let title = self.wrapped(
            &article.title,
            (60.0, Weight::Bold),
            RIGHT - LEFT,
            262.0,
            72.0,
            3,
        );
        let y = self
            .lines(&title, 262.0, 72.0, (60.0, Weight::Bold, FG))
            .max(262.0);
        let by = line(
            &format!("by {}", clean(&article.author, self.measure)),
            34.0,
            Weight::Regular,
            RIGHT - LEFT,
            self.measure,
        );
        self.text(
            LEFT,
            y - 6.0,
            (34.0, Weight::Regular),
            MUTED_FG,
            Anchor::Start,
            &by.text,
        );
        let y = y + 50.0;
        let summary = article
            .summary
            .as_deref()
            .map(|text| self.wrapped(text, (30.0, Weight::Regular), RIGHT - LEFT, y, 42.0, 2))
            .unwrap_or_default();
        self.lines(&summary, y, 42.0, (30.0, Weight::Regular, MUTED_FG));
        Vec::new()
    }

    fn site(&mut self, title: &str, tagline: &str) -> Vec<Count> {
        let title = self.wrapped(title, (84.0, Weight::Bold), RIGHT - LEFT, 300.0, 96.0, 1);
        let y = self.lines(&title, 300.0, 96.0, (84.0, Weight::Bold, FG));
        let tagline = self.wrapped(
            tagline,
            (38.0, Weight::Regular),
            RIGHT - LEFT,
            y - 16.0,
            52.0,
            3,
        );
        self.lines(&tagline, y - 16.0, 52.0, (38.0, Weight::Regular, MUTED_FG));
        Vec::new()
    }
}

/// A repo's footer: stars, followers and, where the page counts them, forks.
fn repo_counts(repo: &Repo) -> Vec<Count> {
    let mut counts = vec![
        Count {
            icon: icon::STAR,
            number: repo.stars,
            singular: "star",
            plural: "stars",
        },
        Count {
            icon: icon::EYE,
            number: repo.followers,
            singular: "follower",
            plural: "followers",
        },
    ];
    if let Some(forks) = repo.forks {
        counts.push(Count {
            icon: icon::FORK,
            number: forks,
            singular: "fork",
            plural: "forks",
        });
    }
    counts
}

/// `1234` as `1.2k`, as a card has room for. A pure function.
fn compact(number: usize) -> String {
    match number {
        0..1_000 => number.to_string(),
        1_000..1_000_000 => trimmed(number as f64 / 1_000.0, "k"),
        _ => trimmed(number as f64 / 1_000_000.0, "M"),
    }
}

fn trimmed(value: f64, unit: &str) -> String {
    let text = if value < 100.0 {
        format!("{:.1}", (value * 10.0).floor() / 10.0)
    } else {
        format!("{}", value.floor())
    };
    format!("{}{unit}", text.trim_end_matches(".0"))
}

/// `text` with XML's five metacharacters escaped.
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::tests::Fixed;

    fn repo() -> Repo {
        Repo {
            owner: "Ada <script>".into(),
            name: "engine".into(),
            description: Some("Computes & tabulates Bernoulli numbers. ".repeat(20)),
            topics: vec!["math".into(), "history".into()],
            stars: 1_234,
            followers: 1,
            forks: Some(0),
        }
    }

    fn texts(svg: &str) -> Vec<String> {
        svg.split("<text ")
            .skip(1)
            .map(|part| {
                let start = part.find('>').unwrap() + 1;
                let end = part.find("</text>").unwrap();
                part[start..end].to_owned()
            })
            .collect()
    }

    #[test]
    fn a_repo_card_shows_owner_name_topics_and_counts_escaped() {
        let svg = draw(
            &Card::new(Mark::Social, Some("relay.test".into()), Face::Repo(repo())),
            &Fixed,
        );
        assert!(svg.starts_with(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1200\" height=\"630\""
        ));
        let shown = texts(&svg);
        for expected in [
            "TOON",
            "Social",
            "AD",
            "Ada &lt;script&gt; /",
            "engine",
            "math",
            "history",
            "1.2k",
            "stars",
            "1",
            "follower",
            "0",
            "forks",
            "relay.test",
        ] {
            assert!(
                shown.iter().any(|text| text == expected),
                "{expected}: {shown:?}"
            );
        }
        assert!(!svg.contains("<script"));
        // The description is cut, and says so.
        let description: Vec<&String> = shown
            .iter()
            .filter(|text| text.starts_with("Computes") || text.contains("Bernoulli"))
            .collect();
        assert!((1..=3).contains(&description.len()), "{description:?}");
        assert!(description.last().unwrap().ends_with('…'));
    }

    #[test]
    fn a_card_draws_no_image_and_no_link() {
        for face in [
            Face::Repo(repo()),
            Face::Ship {
                headline: "v1 is out".into(),
                repo: repo(),
            },
            Face::Profile(Profile {
                name: "Ada".into(),
                nip05: Some("ada@x.y".into()),
                about: Some("Writes engines.".into()),
                repos: 2,
                followers: 5,
                bot: true,
            }),
            Face::Article(Article {
                title: "Notes".into(),
                author: "Ada".into(),
                summary: None,
            }),
            Face::Site {
                title: "TOON Social".into(),
                tagline: "Builders".into(),
            },
        ] {
            let svg = draw(&Card::new(Mark::Repos, None, face), &Fixed);
            for forbidden in [
                "<image",
                "href",
                "<a ",
                "<script",
                "<foreignObject",
                "http://",
                "https://",
            ] {
                let body = svg.replace("http://www.w3.org/2000/svg", "");
                assert!(!body.contains(forbidden), "{forbidden}");
            }
        }
    }

    #[test]
    fn a_profile_card_never_calls_the_claim_verified_and_marks_a_bot() {
        let card = |bot| {
            draw(
                &Card::new(
                    Mark::Social,
                    None,
                    Face::Profile(Profile {
                        name: "Maya".into(),
                        nip05: Some("maya@tide.dev".into()),
                        about: None,
                        repos: 1,
                        followers: 2,
                        bot,
                    }),
                ),
                &Fixed,
            )
        };
        let shown = texts(&card(true));
        assert!(shown.contains(&"maya@tide.dev".to_owned()));
        assert!(shown.contains(&"bot".to_owned()));
        assert!(shown.contains(&"repository".to_owned()));
        assert!(!card(true).to_lowercase().contains("verified"));
        assert!(!texts(&card(false)).contains(&"bot".to_owned()));
    }

    #[test]
    fn a_ship_card_leads_with_its_headline_over_the_repo() {
        let shown = texts(&draw(
            &Card::new(
                Mark::Social,
                None,
                Face::Ship {
                    headline: "Engine 1.0 ships today".into(),
                    repo: repo(),
                },
            ),
            &Fixed,
        ));
        let at = |text: &str| shown.iter().position(|shown| shown == text);
        assert!(at("Shipped").unwrap() < at("Engine 1.0 ships today").unwrap());
        assert!(at("Engine 1.0 ships today").unwrap() < at("engine").unwrap());
    }

    #[test]
    fn counts_are_compact() {
        assert_eq!(compact(0), "0");
        assert_eq!(compact(999), "999");
        assert_eq!(compact(1_000), "1k");
        assert_eq!(compact(1_250), "1.2k");
        assert_eq!(compact(999_999), "999k");
        assert_eq!(compact(12_300_000), "12.3M");
    }

    #[test]
    fn no_line_of_text_sits_below_the_footer() {
        let long = Face::Article(Article {
            title: "word ".repeat(200),
            author: "x".repeat(300),
            summary: Some("y ".repeat(300)),
        });
        let svg = draw(&Card::new(Mark::Social, None, long), &Fixed);
        for part in svg.split("<text x=\"").skip(1) {
            let y: f32 = part
                .split("y=\"")
                .nth(1)
                .unwrap()
                .split('"')
                .next()
                .unwrap()
                .parse()
                .unwrap();
            assert!(y <= FOOTER, "{y}");
        }
    }
}
