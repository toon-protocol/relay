//! An article's highlights (NIP-84, kind 9802): the passages readers saved,
//! grouped by passage and counted by person, and the most-highlighted ones
//! marked where they appear in the article's text. Pure functions, no I/O.
//!
//! A passage is marked only where its text is found word for word inside
//! one text node of the rendered Markdown, outside code, at most once. A
//! passage that crosses formatting (half of it in `*emphasis*`), or that a
//! client trimmed or re-spaced differently, stays in the rail only: finding
//! it would mean matching across elements, which risks marking the wrong
//! words or breaking the markup.

use std::collections::HashSet;

use nostr::event::Event;
use nostr::key::PublicKey;

/// The most passages marked in the text.
pub(crate) const MARKED: usize = 3;
/// A passage is marked only when this many people highlighted it.
pub(crate) const MARK_PEOPLE: usize = 2;
/// A passage shorter than this is never marked: it would match anywhere.
const MARK_CHARS: usize = 12;

/// One passage and who highlighted it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Passage {
    /// The highlighted text, its white space collapsed.
    pub(crate) text: String,
    /// Each person who highlighted it, once, in the order they did.
    pub(crate) people: Vec<PublicKey>,
}

/// `highlights` grouped by passage, the passages most people highlighted
/// first (a tie goes to the one highlighted first). An empty highlight (of
/// media, NIP-84) names no passage and is left out.
pub(crate) fn passages(highlights: &[Event]) -> Vec<Passage> {
    let mut ordered: Vec<&Event> = highlights.iter().collect();
    ordered.sort_by_key(|event| (event.created_at, event.id));
    let mut seen: HashSet<nostr::event::EventId> = HashSet::new();
    let mut found: Vec<Passage> = Vec::new();
    for event in ordered {
        if !seen.insert(event.id) {
            continue;
        }
        let text = collapse(&event.content);
        if text.is_empty() {
            continue;
        }
        match found.iter_mut().find(|passage| passage.text == text) {
            Some(passage) => {
                if !passage.people.contains(&event.pubkey) {
                    passage.people.push(event.pubkey);
                }
            }
            None => found.push(Passage {
                text,
                people: vec![event.pubkey],
            }),
        }
    }
    // A stable sort keeps first-highlighted first among equals.
    found.sort_by_key(|passage| std::cmp::Reverse(passage.people.len()));
    found
}

/// The distinct people who highlighted anything.
pub(crate) fn people(passages: &[Passage]) -> usize {
    passages
        .iter()
        .flat_map(|passage| passage.people.iter())
        .collect::<HashSet<_>>()
        .len()
}

/// `text` with every run of white space one space, trimmed.
fn collapse(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Finds the passages to mark in an article's text nodes, each once.
#[derive(Debug)]
pub(crate) struct Marker {
    /// The passages to mark, most-highlighted first, with their people.
    passages: Vec<(String, usize)>,
    marked: Vec<bool>,
}

impl Marker {
    /// A marker for the [`MARKED`] most-highlighted of `passages` that
    /// [`MARK_PEOPLE`] or more people highlighted.
    pub(crate) fn new(passages: &[Passage]) -> Self {
        let passages: Vec<(String, usize)> = passages
            .iter()
            .filter(|passage| {
                passage.people.len() >= MARK_PEOPLE && passage.text.chars().count() >= MARK_CHARS
            })
            .take(MARKED)
            .map(|passage| (passage.text.clone(), passage.people.len()))
            .collect();
        let marked = vec![false; passages.len()];
        Self { passages, marked }
    }

    /// The passages it would mark, most-highlighted first.
    pub(crate) fn passages(&self) -> impl Iterator<Item = &str> {
        self.passages.iter().map(|(text, _)| text.as_str())
    }

    /// Where in `text` (one text node) to mark, as byte ranges in order,
    /// each with the number of people who highlighted it. A passage found
    /// here is not looked for again. A range never overlaps another and
    /// never cuts a link or `nostr:` mention in two.
    pub(crate) fn ranges(&mut self, text: &str) -> Vec<(usize, usize, usize)> {
        let mut ranges: Vec<(usize, usize, usize)> = Vec::new();
        for (index, (passage, people)) in self.passages.iter().enumerate() {
            if self.marked[index] {
                continue;
            }
            let hit = text.match_indices(passage.as_str()).find_map(|(start, _)| {
                let end = start + passage.len();
                let free = ranges
                    .iter()
                    .all(|(from, to, _)| end <= *from || start >= *to);
                (free && !cuts_a_link(text, start) && !cuts_a_link(text, end))
                    .then_some((start, end))
            });
            if let Some((start, end)) = hit {
                self.marked[index] = true;
                ranges.push((start, end, *people));
            }
        }
        ranges.sort_unstable();
        ranges
    }
}

/// Whether a cut of `text` at byte `at` falls inside a word that is a URL
/// or a `nostr:` mention, which the text's renderer draws as one link.
fn cuts_a_link(text: &str, at: usize) -> bool {
    let (before, after) = text.split_at(at);
    let (Some(left), Some(right)) = (before.chars().next_back(), after.chars().next()) else {
        return false;
    };
    if left.is_whitespace() || right.is_whitespace() {
        return false;
    }
    let start = before
        .rfind(char::is_whitespace)
        .map_or(0, |index| index + 1);
    let end = after
        .find(char::is_whitespace)
        .map_or(text.len(), |index| at + index);
    let word = &text[start..end];
    word.contains("nostr:") || word.contains("://")
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Kind};
    use nostr::key::Keys;
    use nostr::types::Timestamp;

    use super::*;
    use crate::kinds::HIGHLIGHT;

    fn highlight(keys: &Keys, at: u64, text: &str) -> Event {
        EventBuilder::new(Kind::from(HIGHLIGHT), text)
            .custom_created_at(Timestamp::from(at))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn passages_are_counted_by_person_most_first() {
        let (a, b, c) = (Keys::generate(), Keys::generate(), Keys::generate());
        let found = passages(&[
            highlight(&a, 1, "There is no host to go down."),
            highlight(&a, 2, "Cache them for a year."),
            highlight(&a, 3, "Cache them  for a\nyear. "),
            highlight(&b, 4, "Cache them for a year."),
            highlight(&c, 5, ""),
        ]);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].text, "Cache them for a year.");
        assert_eq!(
            found[0].people,
            vec![a.public_key(), b.public_key()],
            "one person twice counts once"
        );
        assert_eq!(found[1].people.len(), 1);
        assert_eq!(people(&found), 2);
    }

    #[test]
    fn only_passages_enough_people_chose_are_marked_once_each() {
        let (a, b) = (Keys::generate(), Keys::generate());
        let found = passages(&[
            highlight(&a, 1, "the old one is never read again"),
            highlight(&b, 2, "the old one is never read again"),
            highlight(&a, 3, "a passage only one person chose"),
            highlight(&a, 4, "short"),
            highlight(&b, 5, "short"),
        ]);
        let mut marker = Marker::new(&found);
        assert_eq!(
            marker.passages().collect::<Vec<_>>(),
            ["the old one is never read again"]
        );
        let text = "So the old one is never read again, and the old one is never read again.";
        assert_eq!(marker.ranges(text), vec![(3, 34, 2)]);
        assert!(
            marker.ranges(text).is_empty(),
            "a passage is marked at most once"
        );
    }

    #[test]
    fn a_mark_never_cuts_a_link_in_two() {
        let (a, b) = (Keys::generate(), Keys::generate());
        let passage = "read the guide at https://x.test";
        let found = passages(&[highlight(&a, 1, passage), highlight(&b, 2, passage)]);
        let mut marker = Marker::new(&found);
        assert!(
            marker
                .ranges("Please read the guide at https://x.test/path today")
                .is_empty()
        );
        assert_eq!(
            marker
                .ranges("Please read the guide at https://x.test today")
                .len(),
            1
        );
        assert!(cuts_a_link("see nostr:npub1abc", 13));
        assert!(!cuts_a_link("see nostr:npub1abc", 4));
        assert!(!cuts_a_link("plain words", 3));
    }
}
