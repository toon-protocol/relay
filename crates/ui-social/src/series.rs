//! A series of articles (proposal 12): a NIP-51 curation set, kind 30004,
//! whose `a` tags name kind 30023 articles in the order the author keeps
//! them. The reader draws, beside an article, each set by the article's
//! author that holds it, with its parts in order and the current one
//! marked. Pure functions, no I/O.
//!
//! NIP-51 (draft) lets a set hold `a` (articles) and `e` (notes) tags and
//! optional `title`, `image` and `description` tags; a set's private items
//! are encrypted in its content, which a reader without the key cannot see,
//! so only the public tags count here.

use nostr::event::Event;
use ui_nip34::repo::{d_tag, first_tag, newest};

use crate::article::ArticleAddress;
use crate::kinds::CURATION_SET;

/// The most parts a series shows.
pub(crate) const MAX_PARTS: usize = 50;

/// One series an article is part of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Series {
    /// The set's `title`, else its `d`.
    pub(crate) title: String,
    /// The set's `description`, if it has one.
    pub(crate) description: Option<String>,
    /// The articles it names, in its order, each once.
    pub(crate) parts: Vec<ArticleAddress>,
    /// Where the article being read is in `parts`.
    pub(crate) current: usize,
}

/// The series among `sets` that hold `article`: the newest version of each
/// kind 30004 set by the article's author, newest set first. A set by
/// anyone else is a curation, not the author's series, and is left out.
pub(crate) fn series_of(sets: &[Event], article: &ArticleAddress) -> Vec<Series> {
    let mut ds: Vec<&str> = Vec::new();
    for set in sets {
        if set.kind.as_u16() == CURATION_SET
            && set.pubkey == *article.author()
            && !ds.contains(&d_tag(set))
        {
            ds.push(d_tag(set));
        }
    }
    let mut found: Vec<(&Event, Series)> = Vec::new();
    for d in ds {
        let Some(set) = newest(sets.iter().filter(|set| {
            set.kind.as_u16() == CURATION_SET && set.pubkey == *article.author() && d_tag(set) == d
        })) else {
            continue;
        };
        let parts = parts(set);
        let Some(current) = parts.iter().position(|part| part == article) else {
            continue;
        };
        let title = first_tag(set, "title")
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or(d)
            .to_owned();
        let description = first_tag(set, "description")
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned);
        found.push((
            set,
            Series {
                title,
                description,
                parts,
                current,
            },
        ));
    }
    found.sort_by(|(a, _), (b, _)| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
    found.into_iter().map(|(_, series)| series).collect()
}

/// The articles a set names in its `a` tags, in order, each once, at most
/// [`MAX_PARTS`].
fn parts(set: &Event) -> Vec<ArticleAddress> {
    let mut parts: Vec<ArticleAddress> = Vec::new();
    for tag in set.tags.iter().map(|tag| tag.as_slice()) {
        if let [name, value, ..] = tag
            && name == "a"
            && let Some(address) = ArticleAddress::parse(value)
            && !parts.contains(&address)
        {
            parts.push(address);
            if parts.len() == MAX_PARTS {
                break;
            }
        }
    }
    parts
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Kind, Tag};
    use nostr::key::Keys;
    use nostr::types::Timestamp;

    use super::*;

    fn set(keys: &Keys, at: u64, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(CURATION_SET), "")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .custom_created_at(Timestamp::from(at))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn a_series_is_the_authors_set_in_its_order_with_the_current_part_marked() {
        let author = Keys::generate();
        let other = Keys::generate();
        let hex = author.public_key().to_hex();
        let part = |d: &str| format!("30023:{hex}:{d}");
        let (one, two, three) = (part("one"), part("two"), part("three"));
        let reading = ArticleAddress::new(author.public_key(), "two");
        let sets = [
            // An older version of the set: replaced by the next one.
            set(&author, 1, &[&["d", "log"], &["a", &two]]),
            set(
                &author,
                2,
                &[
                    &["d", "log"],
                    &["title", "tidepool devlog"],
                    &["a", &one],
                    &["e", &"0".repeat(64)],
                    &["a", &two],
                    &["a", &two],
                    &["a", "30617:x:repo"],
                    &["a", &three],
                ],
            ),
            // Someone else's curation of the same article is not a series.
            set(&other, 3, &[&["d", "picks"], &["a", &two]]),
            // A set of the author's that does not hold it.
            set(&author, 4, &[&["d", "else"], &["a", &one]]),
        ];
        let found = series_of(&sets, &reading);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].title, "tidepool devlog");
        assert_eq!(found[0].current, 1);
        assert_eq!(
            found[0].parts,
            ["one", "two", "three"]
                .map(|d| ArticleAddress::new(author.public_key(), d))
                .to_vec()
        );
    }

    #[test]
    fn a_set_with_no_title_is_called_by_its_d() {
        let author = Keys::generate();
        let reading = ArticleAddress::new(author.public_key(), "a");
        let tag = format!("30023:{}:a", author.public_key().to_hex());
        let found = series_of(
            &[set(&author, 1, &[&["d", "notes"], &["a", &tag]])],
            &reading,
        );
        assert_eq!(found[0].title, "notes");
        assert_eq!(found[0].description, None);
    }
}
