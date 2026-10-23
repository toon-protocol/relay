//! Version order for NIP-82 releases (Appendix D of the draft,
//! nostr-protocol/nips#1336): which of two `version` tags is newer, so an
//! app page shows a channel's newest release by version, not by date.
//!
//! The draft asks for SemVer precedence where a version is SemVer and gives
//! fallback rules otherwise, then lists one sort order as its example:
//! `1.0.0-dev < 1.0.0-alpha < 1.0.0-alpha.2 < 1.0.0-beta < 1.0.0-beta.2 <
//! 1.0.0-rc1 < 1.0.0 < 1.0.1`. Plain SemVer would put `alpha` before `dev`,
//! so one comparator here follows the example, which is the draft's own
//! answer, and agrees with SemVer everywhere the example does not speak:
//!
//! 1. A leading `v` before a digit is dropped and `+build` metadata is
//!    ignored (SemVer gives build metadata no precedence).
//! 2. The rest splits on `.`, `-` and `_` into segments, compared left to
//!    right: numbers as integers of any length, words case-insensitively,
//!    a number before a word.
//! 3. Two words that are both the draft's pre-release identifiers (`dev`,
//!    `develop` < `alpha`, `a` < `beta`, `b` < `rc`, `pre`, `preview`, each
//!    perhaps followed by digits, as in `rc1`) compare by that rank first.
//! 4. When one version runs out: if no word has been passed yet and the
//!    longer one goes on with a word, that word starts a pre-release, so the
//!    longer one is older (`1.0.0-rc1 < 1.0.0`); otherwise the longer one is
//!    newer (`1.0 < 1.0.0`, `1.0.0-alpha < 1.0.0-alpha.2`).

use std::cmp::Ordering;

/// One segment of a version.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment<'a> {
    /// Digits, leading zeros dropped.
    Number(&'a str),
    /// Anything else, as written.
    Word(&'a str),
}

/// The segments of `version`, by rule 1 and 2 of the module.
fn segments(version: &str) -> Vec<Segment<'_>> {
    let version = version.trim();
    let version = version.split('+').next().unwrap_or(version);
    let version = match version.strip_prefix(['v', 'V']) {
        Some(rest) if rest.starts_with(|c: char| c.is_ascii_digit()) => rest,
        _ => version,
    };
    version
        .split(['.', '-', '_'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            if part.bytes().all(|b| b.is_ascii_digit()) {
                let trimmed = part.trim_start_matches('0');
                Segment::Number(if trimmed.is_empty() { "0" } else { trimmed })
            } else {
                Segment::Word(part)
            }
        })
        .collect()
}

/// The draft's pre-release rank of a word, if it is one of its identifiers
/// (perhaps followed by digits).
fn pre_release_rank(word: &str) -> Option<u8> {
    let letters = word.trim_end_matches(|c: char| c.is_ascii_digit());
    match letters.to_ascii_lowercase().as_str() {
        "dev" | "develop" => Some(0),
        "alpha" | "a" => Some(1),
        "beta" | "b" => Some(2),
        "rc" | "pre" | "preview" => Some(3),
        _ => None,
    }
}

fn compare_segments(a: &Segment<'_>, b: &Segment<'_>) -> Ordering {
    match (a, b) {
        (Segment::Number(x), Segment::Number(y)) => x.len().cmp(&y.len()).then_with(|| x.cmp(y)),
        (Segment::Number(_), Segment::Word(_)) => Ordering::Less,
        (Segment::Word(_), Segment::Number(_)) => Ordering::Greater,
        (Segment::Word(x), Segment::Word(y)) => {
            let ranked = match (pre_release_rank(x), pre_release_rank(y)) {
                (Some(r), Some(s)) => r.cmp(&s),
                _ => Ordering::Equal,
            };
            ranked.then_with(|| x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase()))
        }
    }
}

/// How version `a` compares with version `b`, by the module's rules.
/// `Ordering::Greater` means `a` is newer. A pure function.
pub(crate) fn compare(a: &str, b: &str) -> Ordering {
    let (a, b) = (segments(a), segments(b));
    let mut in_pre_release = false;
    for (x, y) in a.iter().zip(&b) {
        match compare_segments(x, y) {
            Ordering::Equal => in_pre_release |= matches!(x, Segment::Word(_)),
            unequal => return unequal,
        }
    }
    let common = a.len().min(b.len());
    let longer_is_newer = |longer: &[Segment<'_>]| {
        in_pre_release || !matches!(longer.get(common), Some(Segment::Word(_)))
    };
    match a.len().cmp(&b.len()) {
        Ordering::Equal => Ordering::Equal,
        Ordering::Greater if longer_is_newer(&a) => Ordering::Greater,
        Ordering::Greater => Ordering::Less,
        Ordering::Less if longer_is_newer(&b) => Ordering::Less,
        Ordering::Less => Ordering::Greater,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ascending(versions: &[&str]) {
        for pair in versions.windows(2) {
            assert_eq!(compare(pair[0], pair[1]), Ordering::Less, "{pair:?}");
            assert_eq!(compare(pair[1], pair[0]), Ordering::Greater, "{pair:?}");
        }
    }

    #[test]
    fn the_drafts_own_example_sorts_as_it_says() {
        ascending(&[
            "1.0.0-dev",
            "1.0.0-alpha",
            "1.0.0-alpha.2",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-rc1",
            "1.0.0",
            "1.0.1",
        ]);
    }

    #[test]
    fn numbers_compare_as_integers_and_missing_segments_are_older() {
        ascending(&["9", "10", "100"]);
        ascending(&["119", "120", "121"]);
        ascending(&["1.0", "1.0.0", "1.0.0.1"]);
        ascending(&["1.9.0", "1.10.0", "2.0.0"]);
        ascending(&["1.0.99999999999999999999998", "1.0.99999999999999999999999"]);
    }

    #[test]
    fn semver_precedence_holds_where_the_example_is_silent() {
        ascending(&[
            "1.0.0-alpha",
            "1.0.0-alpha.1",
            "1.0.0-alpha.beta",
            "1.0.0-beta",
            "1.0.0-beta.2",
            "1.0.0-beta.11",
            "1.0.0-rc.1",
            "1.0.0",
        ]);
        assert_eq!(compare("1.0.0+build.5", "1.0.0"), Ordering::Equal);
    }

    #[test]
    fn a_leading_v_and_case_do_not_matter() {
        assert_eq!(compare("v1.2.0", "1.2.0"), Ordering::Equal);
        assert_eq!(compare("1.2.0-RC1", "1.2.0-rc1"), Ordering::Equal);
        assert_eq!(compare("01.2", "1.2"), Ordering::Equal);
        ascending(&["v1.2.0", "v1.3.0-beta.1", "1.3.0"]);
        ascending(&["0.1.2-rc1", "v0.1.2"]);
        // A word that is not a version prefix is left alone.
        assert_eq!(compare("vintage", "vintage"), Ordering::Equal);
    }

    #[test]
    fn underscores_split_like_dots_and_hyphens() {
        assert_eq!(compare("1_2_3", "1.2.3"), Ordering::Equal);
        ascending(&["1.2.3_beta", "1.2.3"]);
    }
}
