//! A key's activity graph: a year of days, each square the number of
//! **regular** events the key signed that day. Pure functions, no I/O.
//!
//! Only regular events count, because only they are kept: the store holds
//! the newest addressable event per `(pubkey, kind, d)` alone
//! (`docs/retention.md`), so a repo state (kind 30618) that moved a branch
//! a hundred times is one event, and pushes cannot be counted from it. The
//! rule is printed under the graph ([`RULE`]).

use std::collections::HashSet;

use nostr::event::{Event, Kind};
use nostr::key::PublicKey;
use ui_nip34::repo::RepoAddress;

/// NIP-34 patch.
pub(crate) const PATCH: u16 = 1_617;
/// NIP-34 pull request.
pub(crate) const PULL_REQUEST: u16 = 1_618;
/// NIP-34 pull request update.
pub(crate) const PULL_UPDATE: u16 = 1_619;
/// NIP-34 issue.
pub(crate) const ISSUE: u16 = 1_621;
/// NIP-34 status: open.
pub(crate) const STATUS_OPEN: u16 = 1_630;
/// NIP-34 status: applied, merged or resolved.
pub(crate) const STATUS_APPLIED: u16 = 1_631;
/// NIP-34 status: closed.
pub(crate) const STATUS_CLOSED: u16 = 1_632;
/// NIP-34 status: draft.
pub(crate) const STATUS_DRAFT: u16 = 1_633;
/// NIP-22 comment.
pub(crate) const COMMENT: u16 = 1_111;
/// NIP-01 note.
pub(crate) const NOTE: u16 = 1;
/// NIP-C1 (TOON draft) job result.
pub(crate) const CI_JOB: u16 = 9_841;
/// NIP-C1 (TOON draft) workflow result.
pub(crate) const CI_WORKFLOW: u16 = 9_842;

/// The kinds a contribution is.
pub(crate) const COUNTED: [u16; 12] = [
    PATCH,
    PULL_REQUEST,
    PULL_UPDATE,
    ISSUE,
    STATUS_OPEN,
    STATUS_APPLIED,
    STATUS_CLOSED,
    STATUS_DRAFT,
    COMMENT,
    NOTE,
    CI_JOB,
    CI_WORKFLOW,
];

/// The counting rule, as the page prints it.
pub(crate) const RULE: &str = "One square per day; each counts the events this key signed that \
     day of these kinds: patches (1617), pull requests (1618) and their updates (1619), issues \
     (1621), status changes (1630–1633), comments (1111), notes (1) and CI results (9841, 9842). \
     Repo state (30618) is not counted: the relay keeps only the latest one, so pushes cannot be. \
     A repo counts once however often it is named. Darker is busier, relative to this key's \
     busiest day.";

/// Weeks the graph shows.
pub(crate) const WEEKS: i64 = 53;
const DAY: i64 = 86_400;

/// A year of activity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Graph {
    /// The first square's day (days since 1970-01-01), a Sunday.
    start: i64,
    /// Today's day: squares after it are not drawn.
    today: i64,
    /// Per square, column by column (a week a column, Sunday at the top).
    counts: Vec<usize>,
    /// Every counted event.
    pub(crate) total: usize,
    /// Distinct repos the counted events name.
    pub(crate) repos: usize,
}

/// Day `0` was a Thursday; Sunday is `0`.
fn weekday(day: i64) -> i64 {
    (day + 4).rem_euclid(7)
}

/// The first second of the graph that ends at `now`: [`WEEKS`] weeks,
/// starting on a Sunday.
pub(crate) fn since(now: i64) -> i64 {
    let today = now.div_euclid(DAY);
    (today - weekday(today) - (WEEKS - 1) * 7) * DAY
}

/// The graph of `key`'s counted events among `events`, ending at `now`. An
/// event by another key, of another kind or outside the window is left out.
/// A pure function.
pub(crate) fn graph(key: &PublicKey, events: &[Event], now: i64) -> Graph {
    let today = now.div_euclid(DAY);
    let start = since(now).div_euclid(DAY);
    let cells = usize::try_from(WEEKS * 7).unwrap_or(0);
    let mut counts = vec![0; cells];
    let mut total = 0;
    let mut repos: HashSet<RepoAddress> = HashSet::new();
    let mut seen = HashSet::new();
    for event in events {
        if event.pubkey != *key || !COUNTED.contains(&event.kind.as_u16()) || !seen.insert(event.id)
        {
            continue;
        }
        let day = i64::try_from(event.created_at.as_secs())
            .unwrap_or(i64::MAX)
            .div_euclid(DAY);
        if day < start || day > today {
            continue;
        }
        let Some(cell) = usize::try_from(day - start)
            .ok()
            .and_then(|i| counts.get_mut(i))
        else {
            continue;
        };
        *cell += 1;
        total += 1;
        repos.extend(RepoAddress::tagged(event, "a"));
        repos.extend(RepoAddress::tagged(event, "A"));
    }
    Graph {
        start,
        today,
        counts,
        total,
        repos: repos.len(),
    }
}

impl Graph {
    /// Each square in drawing order: its day's first second, its count and
    /// its shade (0 for none, 1–4 by share of the busiest day), or none for
    /// a day after today.
    pub(crate) fn squares(&self) -> impl Iterator<Item = Option<(i64, usize, u8)>> + '_ {
        let busiest = self.counts.iter().copied().max().unwrap_or(0);
        self.counts.iter().enumerate().map(move |(i, count)| {
            let day = self.start + i64::try_from(i).unwrap_or(0);
            (day <= self.today).then(|| (day * DAY, *count, level(*count, busiest)))
        })
    }
}

/// The shade of a day with `count` events when the busiest had `busiest`:
/// none for none, else a quarter step, rounded up. A pure function.
pub(crate) fn level(count: usize, busiest: usize) -> u8 {
    if count == 0 || busiest == 0 {
        return 0;
    }
    let step = (count * 4).div_ceil(busiest).clamp(1, 4);
    u8::try_from(step).unwrap_or(4)
}

/// What the recent-activity list calls an event.
pub(crate) fn label(event: &Event) -> &'static str {
    match event.kind.as_u16() {
        PATCH => "Patch",
        PULL_REQUEST => "Pull request",
        PULL_UPDATE => "PR update",
        ISSUE => "Issue",
        STATUS_OPEN => "Opened",
        STATUS_APPLIED => "Merged",
        STATUS_CLOSED => "Closed",
        STATUS_DRAFT => "Draft",
        COMMENT => "Comment",
        NOTE if crate::is_reply(event) => "Reply",
        NOTE => "Note",
        CI_JOB => "CI job",
        CI_WORKFLOW => "CI workflow",
        _ => "Event",
    }
}

/// Whether `kind` is a NIP-34 status.
pub(crate) fn is_status(kind: Kind) -> bool {
    (STATUS_OPEN..=STATUS_DRAFT).contains(&kind.as_u16())
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;
    use nostr::types::Timestamp;

    use super::*;

    fn signed(keys: &Keys, kind: u16, at: i64, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .custom_created_at(Timestamp::from(u64::try_from(at).unwrap()))
            .finalize(keys)
            .unwrap()
    }

    // Wednesday, Oct 21, 2026, at noon.
    const NOW: i64 = 1_792_540_800 + 43_200;

    #[test]
    fn the_graph_is_53_weeks_from_a_sunday_ending_today() {
        let start = since(NOW);
        assert_eq!(weekday(start / DAY), 0, "it starts on a Sunday");
        assert_eq!(weekday(NOW / DAY), 3, "Oct 21, 2026 is a Wednesday");
        let keys = Keys::generate();
        let drawn = graph(&keys.public_key(), &[], NOW);
        let squares: Vec<_> = drawn.squares().collect();
        assert_eq!(squares.len(), 53 * 7);
        assert_eq!(
            squares.iter().filter(|square| square.is_none()).count(),
            3,
            "Thursday to Saturday of this week are still to come"
        );
        assert_eq!(drawn.total, 0);
    }

    #[test]
    fn only_this_keys_regular_contributions_in_the_window_count() {
        let (me, other) = (Keys::generate(), Keys::generate());
        let hex = me.public_key().to_hex();
        let tidepool = format!("30617:{hex}:tidepool");
        let gitsail = format!("30617:{}:gitsail", other.public_key().to_hex());
        let events = vec![
            signed(&me, PULL_REQUEST, NOW - 10, &[&["a", &gitsail]]),
            signed(&me, STATUS_APPLIED, NOW - 20, &[&["a", &tidepool]]),
            signed(&me, COMMENT, NOW - 30, &[&["A", &tidepool]]),
            signed(&me, NOTE, NOW - 2 * DAY, &[]),
            // Not counted: repo state, another key, before the window.
            signed(&me, 30_618, NOW - 40, &[&["d", "tidepool"]]),
            signed(&other, ISSUE, NOW - 50, &[&["a", &gitsail]]),
            signed(&me, ISSUE, since(NOW) - 1, &[&["a", &gitsail]]),
        ];
        let mut doubled = events.clone();
        doubled.push(events[0].clone());
        let drawn = graph(&me.public_key(), &doubled, NOW);
        assert_eq!(drawn.total, 4);
        assert_eq!(drawn.repos, 2);
        let today: Vec<_> = drawn.squares().flatten().collect();
        assert_eq!(
            today.last().map(|(_, count, shade)| (*count, *shade)),
            Some((3, 4))
        );
        assert_eq!(today[today.len() - 3].1, 1);
    }

    #[test]
    fn shades_are_quarters_of_the_busiest_day() {
        assert_eq!(level(0, 8), 0);
        assert_eq!(level(1, 8), 1);
        assert_eq!(level(2, 8), 1);
        assert_eq!(level(3, 8), 2);
        assert_eq!(level(6, 8), 3);
        assert_eq!(level(8, 8), 4);
        assert_eq!(level(1, 1), 4);
        assert_eq!(level(0, 0), 0);
    }
}
