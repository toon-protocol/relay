//! Finding repositories in what the list page read: a pure function of the
//! announcements and their owners' names. Every word of a query must match
//! somewhere; where it matched decides the order. The store has no text index
//! and needs none at this size, so this runs over the list's own read.

use std::collections::HashMap;

use nostr::event::Event;
use nostr::key::PublicKey;
use relay_ui::chrome::{escape, npub};

use crate::repo::{d_tag, first_tag, repo_name};

/// The longest query read; the rest is ignored.
const QUERY_LIMIT: usize = 200;

/// How a list of repositories is ordered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Sort {
    Best,
    Updated,
    Name,
}

impl Sort {
    /// The order `name` asks for: best match when there is a query to match,
    /// else by name.
    pub(crate) fn asked(name: Option<&str>, searching: bool) -> Self {
        match name {
            Some("updated") => Self::Updated,
            Some("name") => Self::Name,
            _ if searching => Self::Best,
            _ => Self::Name,
        }
    }
}

/// A query's lower-case words.
pub(crate) fn words(query: &str) -> Vec<String> {
    query
        .chars()
        .take(QUERY_LIMIT)
        .collect::<String>()
        .to_lowercase()
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

/// A repository's lower-case topics, its `t` tags.
pub(crate) fn topics(repo: &Event) -> Vec<String> {
    repo.tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter(|tag| tag.first().is_some_and(|name| name == "t"))
        .filter_map(|tag| tag.get(1))
        .map(|topic| topic.to_lowercase())
        .filter(|topic| !topic.is_empty())
        .collect()
}

/// Whether `word` starts a word of `text`, not only sits inside one.
fn starts_a_word(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(at, _)| {
        !text[..at]
            .chars()
            .next_back()
            .is_some_and(char::is_alphanumeric)
    })
}

/// How well one lower-case `word` matches a repository; 0 is no match.
fn word_score(word: &str, repo: &Event, owner: &str, key: &PublicKey) -> u32 {
    let mut best = 0;
    for name in [repo_name(repo).to_lowercase(), d_tag(repo).to_lowercase()] {
        best = best.max(if name == word {
            100
        } else if name.starts_with(word) {
            60
        } else if starts_a_word(&name, word) {
            40
        } else if name.contains(word) {
            25
        } else {
            0
        });
    }
    for topic in topics(repo) {
        best = best.max(if topic == word {
            30
        } else if topic.contains(word) {
            15
        } else {
            0
        });
    }
    if owner.to_lowercase().contains(word)
        || npub(key).starts_with(word)
        || key.to_hex().starts_with(word)
    {
        best = best.max(15);
    }
    let description = first_tag(repo, "description")
        .unwrap_or_default()
        .to_lowercase();
    if starts_a_word(&description, word) {
        best = best.max(10);
    } else if description.contains(word) {
        best = best.max(5);
    }
    best
}

/// How well `words` match a repository: `None` unless every one does.
fn score(words: &[String], repo: &Event, names: &HashMap<PublicKey, String>) -> Option<u32> {
    let owner = names.get(&repo.pubkey).map_or("", String::as_str);
    words.iter().try_fold(0, |total, word| {
        match word_score(word, repo, owner, &repo.pubkey) {
            0 => None,
            found => Some(total + found),
        }
    })
}

/// The `repos` that match `words` and carry `topic`, in `sort`'s order.
pub(crate) fn find<'a>(
    repos: &[&'a Event],
    names: &HashMap<PublicKey, String>,
    words: &[String],
    topic: Option<&str>,
    sort: Sort,
) -> Vec<&'a Event> {
    let mut found: Vec<(u32, &Event)> = repos
        .iter()
        .filter(|repo| topic.is_none_or(|topic| topics(repo).iter().any(|own| own == topic)))
        .filter_map(|repo| Some((score(words, repo, names)?, *repo)))
        .collect();
    let by_name = |a: &Event, b: &Event| {
        (repo_name(a).to_lowercase(), a.pubkey.to_hex())
            .cmp(&(repo_name(b).to_lowercase(), b.pubkey.to_hex()))
    };
    match sort {
        Sort::Best => found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| by_name(a.1, b.1))),
        Sort::Updated => found.sort_by(|a, b| {
            b.1.created_at
                .cmp(&a.1.created_at)
                .then_with(|| by_name(a.1, b.1))
        }),
        Sort::Name => found.sort_by(|a, b| by_name(a.1, b.1)),
    }
    found.into_iter().map(|(_, repo)| repo).collect()
}

/// `text`, escaped, with each place a lower-case word matches in `<mark>`.
pub(crate) fn highlight(text: &str, words: &[String]) -> String {
    let chars: Vec<char> = text.chars().collect();
    let folded: Vec<char> = chars
        .iter()
        .map(|c| c.to_lowercase().next().unwrap_or(*c))
        .collect();
    let mut marked = vec![false; chars.len()];
    for word in words {
        let word: Vec<char> = word.chars().collect();
        if word.is_empty() || word.len() > folded.len() {
            continue;
        }
        for start in 0..=folded.len() - word.len() {
            if folded[start..start + word.len()] == word[..] {
                marked[start..start + word.len()].fill(true);
            }
        }
    }
    let mut out = String::new();
    let mut at = 0;
    while at < chars.len() {
        let run = marked[at..]
            .iter()
            .take_while(|mark| **mark == marked[at])
            .count();
        let piece: String = chars[at..at + run].iter().collect();
        if marked[at] {
            out.push_str(&format!("<mark>{}</mark>", escape(&piece)));
        } else {
            out.push_str(&escape(&piece));
        }
        at += run;
    }
    out
}

/// The most used topics of `repos`, most used first, then by name.
pub(crate) fn top_topics(repos: &[&Event], limit: usize) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for repo in repos {
        for topic in topics(repo) {
            *counts.entry(topic).or_default() += 1;
        }
    }
    let mut topics: Vec<(String, usize)> = counts.into_iter().collect();
    topics.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    topics.truncate(limit);
    topics
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Kind, Tag};
    use nostr::key::Keys;

    use super::*;

    fn repo(d: &str, description: &str, topics: &[&str]) -> Event {
        let mut tags = vec![
            Tag::parse(["d", d]).unwrap(),
            Tag::parse(["description", description]).unwrap(),
        ];
        tags.extend(
            topics
                .iter()
                .map(|topic| Tag::parse(["t", *topic]).unwrap()),
        );
        EventBuilder::new(Kind::from(30617), "")
            .tags(tags)
            .finalize(&Keys::generate())
            .unwrap()
    }

    fn found(repos: &[Event], query: &str, topic: Option<&str>, sort: Sort) -> Vec<String> {
        let refs: Vec<&Event> = repos.iter().collect();
        find(&refs, &HashMap::new(), &words(query), topic, sort)
            .into_iter()
            .map(|repo| d_tag(repo).to_owned())
            .collect()
    }

    #[test]
    fn a_name_outranks_a_topic_and_a_topic_outranks_a_description() {
        let repos = [
            repo("about", "written in rust", &[]),
            repo("tagged", "", &["rust"]),
            repo("rust", "", &[]),
            repo("rustic", "", &[]),
        ];
        assert_eq!(
            found(&repos, "RUST", None, Sort::Best),
            ["rust", "rustic", "tagged", "about"]
        );
    }

    #[test]
    fn every_word_must_match_and_nothing_else_is_returned() {
        let repos = [repo("relay", "a nostr relay", &[]), repo("relic", "", &[])];
        assert_eq!(found(&repos, "relay nostr", None, Sort::Best), ["relay"]);
        assert!(found(&repos, "relay missing", None, Sort::Best).is_empty());
    }

    #[test]
    fn an_empty_query_keeps_everything_and_a_topic_narrows_it() {
        let repos = [repo("a", "", &["Rust"]), repo("b", "", &["go"])];
        assert_eq!(found(&repos, "  ", None, Sort::Name), ["a", "b"]);
        assert_eq!(found(&repos, "", Some("rust"), Sort::Name), ["a"]);
    }

    #[test]
    fn an_owner_is_found_by_name_or_npub() {
        let repos = [repo("a", "", &[])];
        let refs: Vec<&Event> = repos.iter().collect();
        let key = repos[0].pubkey;
        let names = HashMap::from([(key, "Ada Lovelace".to_owned())]);
        assert_eq!(
            find(&refs, &names, &words("lovelace"), None, Sort::Best).len(),
            1
        );
        assert_eq!(
            find(&refs, &names, &words(&npub(&key)), None, Sort::Best).len(),
            1
        );
    }

    #[test]
    fn a_highlight_marks_any_case_and_escapes_the_rest() {
        assert_eq!(
            highlight("<Relay> relay", &words("relay")),
            "&lt;<mark>Relay</mark>&gt; <mark>relay</mark>"
        );
        assert_eq!(highlight("plain", &[]), "plain");
    }
}
