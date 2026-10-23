//! NIP-52 calendar events, read from their events: what a date-based
//! (kind 31922) or time-based (kind 31923) calendar event says, when it
//! happens, which phase it is in now, who hosts it, and who RSVP'd (kind
//! 31925). Pure functions over events, with no I/O, so the events index, an
//! event's page and the feed's "Coming up" box read an event the same way.
//!
//! NIP-52 is a draft. What this module adds to it is reading, not wire
//! shapes: a time is shown in UTC (the page has no time-zone database, so a
//! `start_tzid` is named beside it, not applied), and a host is the event's
//! author or a `p` whose role says host or organizer.

use std::collections::HashMap;

use nostr::event::{Event, Kind};
use nostr::key::PublicKey;
use nostr::nips::nip01::Coordinate;
use nostr::nips::nip19::{Nip19Coordinate, ToBech32};
use relay_ui::chrome::{self, npub, percent_encode};
use ui_nip34::repo::{d_tag, first_tag};

use crate::PREFIX;

/// NIP-52 date-based calendar event.
pub(crate) const DATE_EVENT: u16 = 31_922;
/// NIP-52 time-based calendar event.
pub(crate) const TIME_EVENT: u16 = 31_923;
/// NIP-52 calendar event RSVP.
pub(crate) const RSVP: u16 = 31_925;

const DAY: i64 = 86_400;

/// When a calendar event happens.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum When {
    /// Whole days, UTC: `start` inclusive, `end` exclusive (days since the
    /// epoch). A 31922 without an `end` is the one day of its `start`.
    Days { start: i64, end: i64 },
    /// A span of Unix seconds. A 31923 without an `end` (or with one not
    /// after its `start`) ends the moment it starts.
    Time {
        start: i64,
        end: Option<i64>,
        zone: Option<String>,
    },
}

/// Where a calendar event stands at a moment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    Before,
    Live,
    Ended,
}

/// A calendar event, read.
#[derive(Debug, Clone)]
pub(crate) struct CalendarEvent {
    event: Event,
    title: String,
    when: When,
}

impl CalendarEvent {
    /// `event` as a calendar event, if it is a 31922 or 31923 with a `d`, a
    /// title (`title`, else the deprecated `name`) and a `start` its kind can
    /// read.
    pub(crate) fn read(event: &Event) -> Option<Self> {
        if d_tag(event).is_empty() {
            return None;
        }
        let title = first_tag(event, "title")
            .or_else(|| first_tag(event, "name"))
            .map(str::trim)
            .filter(|title| !title.is_empty())?
            .to_owned();
        let start = first_tag(event, "start")?;
        let end = first_tag(event, "end");
        let when = match event.kind.as_u16() {
            DATE_EVENT => {
                let start = day_number(start)?;
                let end = end
                    .and_then(day_number)
                    .filter(|end| *end > start)
                    .unwrap_or(start + 1);
                When::Days { start, end }
            }
            TIME_EVENT => {
                let start = start.trim().parse::<i64>().ok()?;
                let end = end
                    .and_then(|end| end.trim().parse::<i64>().ok())
                    .filter(|end| *end > start);
                let zone = first_tag(event, "start_tzid")
                    .map(str::trim)
                    .filter(|zone| !zone.is_empty())
                    .map(str::to_owned);
                When::Time { start, end, zone }
            }
            _ => return None,
        };
        Some(Self {
            event: event.clone(),
            title,
            when,
        })
    }

    pub(crate) fn event(&self) -> &Event {
        &self.event
    }

    pub(crate) fn title(&self) -> &str {
        &self.title
    }

    pub(crate) fn author(&self) -> &PublicKey {
        &self.event.pubkey
    }

    pub(crate) fn d(&self) -> &str {
        d_tag(&self.event)
    }

    /// `<kind>:<author hex>:<d>`, as an `a` tag names it.
    pub(crate) fn coordinate(&self) -> String {
        format!(
            "{}:{}:{}",
            self.event.kind.as_u16(),
            self.event.pubkey.to_hex(),
            self.d()
        )
    }

    /// The `naddr` of the event, for a `nostr:` link a client opens.
    pub(crate) fn naddr(&self) -> String {
        let coordinate = Coordinate::new(self.event.kind, self.event.pubkey).identifier(self.d());
        Nip19Coordinate::new(coordinate, Vec::<nostr::types::RelayUrl>::new())
            .to_bech32()
            .unwrap_or_default()
    }

    /// The event's page.
    pub(crate) fn path(&self) -> String {
        format!(
            "{PREFIX}/ev/{}/{}",
            npub(&self.event.pubkey),
            percent_encode(self.d())
        )
    }

    /// When it starts, in Unix seconds.
    pub(crate) fn starts_at(&self) -> i64 {
        match self.when {
            When::Days { start, .. } => start * DAY,
            When::Time { start, .. } => start,
        }
    }

    /// When it is over, in Unix seconds (exclusive).
    pub(crate) fn ends_at(&self) -> i64 {
        match self.when {
            When::Days { end, .. } => end * DAY,
            When::Time { start, end, .. } => end.unwrap_or(start),
        }
    }

    /// Its phase at `now`. A pure function.
    pub(crate) fn phase(&self, now: i64) -> Phase {
        if now < self.starts_at() {
            Phase::Before
        } else if now < self.ends_at() {
            Phase::Live
        } else {
            Phase::Ended
        }
    }

    /// The `summary` tag, else the start of the content.
    pub(crate) fn summary(&self) -> Option<String> {
        first_tag(&self.event, "summary")
            .map(str::trim)
            .filter(|summary| !summary.is_empty())
            .map(str::to_owned)
            .or_else(|| {
                let line = crate::content::first_line(&self.event.content, 200);
                (!line.is_empty()).then_some(line)
            })
    }

    /// Every `location` tag's value.
    pub(crate) fn locations(&self) -> Vec<&str> {
        self.values("location")
    }

    /// Every `t` tag's value.
    pub(crate) fn topics(&self) -> Vec<&str> {
        self.values("t")
    }

    fn values(&self, name: &str) -> Vec<&str> {
        self.event
            .tags
            .iter()
            .map(|tag| tag.as_slice())
            .filter_map(|tag| match tag {
                [tag_name, value, ..] if tag_name == name && !value.trim().is_empty() => {
                    Some(value.trim())
                }
                _ => None,
            })
            .collect()
    }

    /// The `p` tags, once each, with the role each names (the fourth value).
    pub(crate) fn participants(&self) -> Vec<(PublicKey, Option<String>)> {
        let mut found: Vec<(PublicKey, Option<String>)> = Vec::new();
        for tag in self.event.tags.iter().map(|tag| tag.as_slice()) {
            let [name, key, rest @ ..] = tag else {
                continue;
            };
            if name != "p" {
                continue;
            }
            let Ok(key) = PublicKey::from_hex(key) else {
                continue;
            };
            if found.iter().any(|(held, _)| *held == key) {
                continue;
            }
            let role = rest
                .get(1)
                .map(|role| role.trim())
                .filter(|role| !role.is_empty())
                .map(str::to_owned);
            found.push((key, role));
        }
        found
    }

    /// The hosts: the author, then each `p` whose role names a host or an
    /// organizer. Their keys are the ones whose badges count. A pure function.
    pub(crate) fn hosts(&self) -> Vec<PublicKey> {
        let mut hosts = vec![self.event.pubkey];
        for (key, role) in self.participants() {
            if role.as_deref().is_some_and(is_host_role) && !hosts.contains(&key) {
                hosts.push(key);
            }
        }
        hosts
    }

    /// The NIP-29 group the event was posted to (its `h` tag): its room.
    pub(crate) fn room(&self) -> Option<&str> {
        first_tag(&self.event, "h").filter(|id| !id.is_empty())
    }

    /// When, in words: `Sat, Nov 2, 2026, 09:00 – 18:00 UTC`, or the days of
    /// a date-based event. A pure function.
    pub(crate) fn time_label(&self) -> String {
        match &self.when {
            When::Days { start, end } => {
                let first = day_label(*start);
                if *end == start + 1 {
                    format!("{first} (all day)")
                } else {
                    format!("{first} – {} (all days)", day_label(end - 1))
                }
            }
            When::Time { start, end, zone } => {
                let mut out = format!("{}, {}", day_label(start.div_euclid(DAY)), clock(*start));
                if let Some(end) = end {
                    if end.div_euclid(DAY) == start.div_euclid(DAY) {
                        out.push_str(&format!(" – {}", clock(*end)));
                    } else {
                        out.push_str(&format!(
                            " – {}, {}",
                            day_label(end.div_euclid(DAY)),
                            clock(*end)
                        ));
                    }
                }
                out.push_str(" UTC");
                if let Some(zone) = zone
                    && zone != "UTC"
                    && zone != "Etc/UTC"
                {
                    out.push_str(&format!(" (the host's time zone is {zone})"));
                }
                out
            }
        }
    }

    /// `(MON, day)` for a date badge, in UTC.
    pub(crate) fn badge_date(&self) -> (String, String) {
        let date = chrome::date(self.starts_at());
        let mut parts = date.split([' ', ',']).filter(|part| !part.is_empty());
        let month = parts.next().unwrap_or("").to_uppercase();
        let day = parts.next().unwrap_or("").to_owned();
        (month, day)
    }
}

/// Whether a `p` role names a host: `host`, `co-host`, `organizer`. A pure
/// function.
pub(crate) fn is_host_role(role: &str) -> bool {
    let role = role.to_ascii_lowercase();
    role.contains("host") || role.contains("organizer") || role.contains("organiser")
}

/// `Sat, Nov 2, 2026` for a day number.
fn day_label(day: i64) -> String {
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    let weekday = WEEKDAYS[usize::try_from(day.rem_euclid(7)).unwrap_or(0)];
    format!("{weekday}, {}", chrome::date(day * DAY))
}

/// `09:05` for a Unix time, UTC.
fn clock(seconds: i64) -> String {
    let of_day = seconds.rem_euclid(DAY);
    format!("{:02}:{:02}", of_day / 3_600, (of_day % 3_600) / 60)
}

/// The day number (days since 1970-01-01) of `YYYY-MM-DD`, if it is one. A
/// pure function.
pub(crate) fn day_number(text: &str) -> Option<i64> {
    let text = text.trim();
    let mut parts = text.splitn(3, '-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    if year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return None;
    }
    let (year, month, day) = (
        year.parse::<i64>().ok()?,
        month.parse::<i64>().ok()?,
        day.parse::<i64>().ok()?,
    );
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    let days_in_month = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return None,
    };
    if !(1..=days_in_month).contains(&day) {
        return None;
    }
    // Howard Hinnant's days_from_civil.
    let y = if month <= 2 { year - 1 } else { year };
    let era = y.div_euclid(400);
    let yoe = y - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * 146_097 + doe - 719_468)
}

/// How long until (or for) `seconds`, roughly: `40 minutes`, `5 hours`,
/// `9 days`. A pure function.
pub(crate) fn span(seconds: i64) -> String {
    let seconds = seconds.max(0);
    let plural = |n: i64, unit: &str| format!("{n} {unit}{}", if n == 1 { "" } else { "s" });
    match seconds {
        0..3_600 => plural((seconds / 60).max(1), "minute"),
        3_600..172_800 => plural(seconds / 3_600, "hour"),
        _ => plural(seconds / DAY, "day"),
    }
}

/// The newest of each calendar event among `events` (one per kind, author
/// and `d`), read. A pure function.
pub(crate) fn newest_each(events: &[Event]) -> Vec<CalendarEvent> {
    let mut held: HashMap<(Kind, PublicKey, String), &Event> = HashMap::new();
    for event in events {
        let key = (event.kind, event.pubkey, d_tag(event).to_owned());
        match held.get(&key) {
            Some(current) if !ui_nip34::repo::supersedes(event, current) => {}
            _ => {
                held.insert(key, event);
            }
        }
    }
    held.into_values().filter_map(CalendarEvent::read).collect()
}

/// Calendar events split by phase at `now`: live ones ending soonest first,
/// upcoming ones starting soonest first, past ones most recent first. A pure
/// function.
pub(crate) fn schedule(
    events: Vec<CalendarEvent>,
    now: i64,
) -> (Vec<CalendarEvent>, Vec<CalendarEvent>, Vec<CalendarEvent>) {
    let (mut live, mut upcoming, mut past) = (Vec::new(), Vec::new(), Vec::new());
    for event in events {
        match event.phase(now) {
            Phase::Live => live.push(event),
            Phase::Before => upcoming.push(event),
            Phase::Ended => past.push(event),
        }
    }
    let order = |a: &CalendarEvent, b: &CalendarEvent| a.coordinate().cmp(&b.coordinate());
    live.sort_by(|a, b| a.ends_at().cmp(&b.ends_at()).then_with(|| order(a, b)));
    upcoming.sort_by(|a, b| a.starts_at().cmp(&b.starts_at()).then_with(|| order(a, b)));
    past.sort_by(|a, b| b.ends_at().cmp(&a.ends_at()).then_with(|| order(a, b)));
    (live, upcoming, past)
}

/// What RSVPs say about one calendar event: distinct people by their newest
/// answer.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct Rsvps {
    pub(crate) accepted: usize,
    pub(crate) tentative: usize,
    pub(crate) declined: usize,
}

/// The RSVPs (kind 31925) among `events` whose `a` tag is `coordinate`,
/// counted by person: each key's newest answer is the one that counts,
/// however many RSVPs it wrote. A pure function.
pub(crate) fn rsvps(events: &[Event], coordinate: &str) -> Rsvps {
    let mut newest: HashMap<PublicKey, &Event> = HashMap::new();
    for event in events {
        if event.kind != Kind::from(RSVP) {
            continue;
        }
        let names = event
            .tags
            .iter()
            .map(|tag| tag.as_slice())
            .any(|tag| matches!(tag, [name, value, ..] if name == "a" && value == coordinate));
        if !names {
            continue;
        }
        match newest.get(&event.pubkey) {
            Some(held) if !ui_nip34::repo::supersedes(event, held) => {}
            _ => {
                newest.insert(event.pubkey, event);
            }
        }
    }
    let mut counts = Rsvps::default();
    for event in newest.values() {
        match first_tag(event, "status")
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("accepted") => counts.accepted += 1,
            Some("tentative") => counts.tentative += 1,
            Some("declined") => counts.declined += 1,
            _ => {}
        }
    }
    counts
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;
    use nostr::types::Timestamp;

    use super::*;

    fn signed(keys: &Keys, kind: u16, at: u64, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .custom_created_at(Timestamp::from(at))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn a_date_reads_as_its_day_number() {
        assert_eq!(day_number("1970-01-01"), Some(0));
        assert_eq!(day_number("2026-10-21"), Some(1_792_540_800 / DAY));
        assert_eq!(day_number("2024-02-29"), Some(19_782));
        for bad in ["2023-02-29", "2026-13-01", "26-01-01", "2026-1-1", "soon"] {
            assert_eq!(day_number(bad), None, "{bad}");
        }
    }

    #[test]
    fn a_time_event_is_before_live_and_ended_by_its_start_and_end() {
        let keys = Keys::generate();
        let event = signed(
            &keys,
            TIME_EVENT,
            1,
            &[
                &["d", "jam"],
                &["title", "Jam"],
                &["start", "1000"],
                &["end", "2000"],
            ],
        );
        let jam = CalendarEvent::read(&event).unwrap();
        assert_eq!(jam.phase(999), Phase::Before);
        assert_eq!(jam.phase(1000), Phase::Live);
        assert_eq!(jam.phase(1999), Phase::Live);
        assert_eq!(jam.phase(2000), Phase::Ended, "the end is exclusive");

        let instant = signed(
            &keys,
            TIME_EVENT,
            1,
            &[
                &["d", "i"],
                &["title", "I"],
                &["start", "1000"],
                &["end", "900"],
            ],
        );
        let instant = CalendarEvent::read(&instant).unwrap();
        assert_eq!(
            instant.phase(1000),
            Phase::Ended,
            "an end before the start is none"
        );
    }

    #[test]
    fn a_date_event_lasts_whole_days_and_one_day_without_an_end() {
        let keys = Keys::generate();
        let one = signed(
            &keys,
            DATE_EVENT,
            1,
            &[&["d", "x"], &["name", "Old name"], &["start", "2026-11-02"]],
        );
        let one = CalendarEvent::read(&one).unwrap();
        assert_eq!(one.title(), "Old name", "the deprecated name stands in");
        let start = day_number("2026-11-02").unwrap() * DAY;
        assert_eq!(one.phase(start - 1), Phase::Before);
        assert_eq!(one.phase(start + DAY - 1), Phase::Live);
        assert_eq!(one.phase(start + DAY), Phase::Ended);
        assert_eq!(one.time_label(), "Mon, Nov 2, 2026 (all day)");

        let two = signed(
            &keys,
            DATE_EVENT,
            1,
            &[
                &["d", "y"],
                &["title", "Y"],
                &["start", "2026-11-02"],
                &["end", "2026-11-04"],
            ],
        );
        let two = CalendarEvent::read(&two).unwrap();
        assert_eq!(
            two.time_label(),
            "Mon, Nov 2, 2026 – Tue, Nov 3, 2026 (all days)",
            "the end date is exclusive"
        );
        assert!(
            CalendarEvent::read(&signed(
                &keys,
                DATE_EVENT,
                1,
                &[&["d", "z"], &["title", "Z"]]
            ))
            .is_none(),
            "a start is required"
        );
    }

    #[test]
    fn a_time_is_shown_in_utc_beside_the_hosts_zone() {
        let keys = Keys::generate();
        let start = day_number("2026-11-02").unwrap() * DAY + 9 * 3_600;
        let (start_text, same_day, next_day) = (
            start.to_string(),
            (start + 9 * 3_600).to_string(),
            (start + 33 * 3_600).to_string(),
        );
        let label = |end: &str, zone: &str| {
            let event = signed(
                &keys,
                TIME_EVENT,
                1,
                &[
                    &["d", "x"],
                    &["title", "X"],
                    &["start", &start_text],
                    &["end", end],
                    &["start_tzid", zone],
                ],
            );
            CalendarEvent::read(&event).unwrap().time_label()
        };
        assert_eq!(
            label(&same_day, "UTC"),
            "Mon, Nov 2, 2026, 09:00 – 18:00 UTC"
        );
        assert_eq!(
            label(&next_day, "Europe/Berlin"),
            "Mon, Nov 2, 2026, 09:00 – Tue, Nov 3, 2026, 18:00 UTC (the host's time zone is Europe/Berlin)"
        );
    }

    #[test]
    fn hosts_are_the_author_and_the_p_tags_whose_role_says_so() {
        let (author, cohost, judge) = (Keys::generate(), Keys::generate(), Keys::generate());
        let event = signed(
            &author,
            TIME_EVENT,
            1,
            &[
                &["d", "x"],
                &["title", "X"],
                &["start", "1"],
                &["p", &cohost.public_key().to_hex(), "", "Co-Host"],
                &["p", &judge.public_key().to_hex(), "wss://r", "judge"],
                &["p", &judge.public_key().to_hex(), "", "host"],
                &["p", "not-a-key", "", "host"],
            ],
        );
        let event = CalendarEvent::read(&event).unwrap();
        assert_eq!(
            event.hosts(),
            vec![author.public_key(), cohost.public_key()]
        );
        assert_eq!(
            event.participants().len(),
            2,
            "each key once, its first role"
        );
    }

    #[test]
    fn rsvps_count_each_persons_newest_answer() {
        let (a, b, c) = (Keys::generate(), Keys::generate(), Keys::generate());
        let target = "31923:abc:jam";
        let rsvp = |keys: &Keys, at: u64, d: &str, status: &str| {
            signed(
                keys,
                RSVP,
                at,
                &[&["d", d], &["a", target], &["status", status]],
            )
        };
        let events = [
            rsvp(&a, 1, "1", "tentative"),
            rsvp(&a, 2, "2", "accepted"),
            rsvp(&a, 3, "3", "accepted"),
            rsvp(&b, 1, "1", "tentative"),
            rsvp(&c, 5, "1", "accepted"),
            rsvp(&c, 6, "1", "declined"),
            signed(
                &c,
                RSVP,
                9,
                &[
                    &["d", "o"],
                    &["a", "31923:abc:other"],
                    &["status", "accepted"],
                ],
            ),
        ];
        assert_eq!(
            rsvps(&events, target),
            Rsvps {
                accepted: 1,
                tentative: 1,
                declined: 1
            }
        );
    }

    #[test]
    fn the_schedule_puts_the_soonest_first() {
        let keys = Keys::generate();
        let at = |d: &str, start: i64, end: i64| {
            let (start, end) = (start.to_string(), end.to_string());
            CalendarEvent::read(&signed(
                &keys,
                TIME_EVENT,
                1,
                &[&["d", d], &["title", d], &["start", &start], &["end", &end]],
            ))
            .unwrap()
        };
        let (live, upcoming, past) = schedule(
            vec![
                at("later", 300, 400),
                at("soon", 200, 400),
                at("now", 50, 150),
                at("old", 1, 10),
                at("older", 1, 5),
            ],
            100,
        );
        assert_eq!(live.iter().map(|e| e.d()).collect::<Vec<_>>(), ["now"]);
        assert_eq!(
            upcoming.iter().map(|e| e.d()).collect::<Vec<_>>(),
            ["soon", "later"]
        );
        assert_eq!(
            past.iter().map(|e| e.d()).collect::<Vec<_>>(),
            ["old", "older"]
        );
    }

    #[test]
    fn a_span_reads_in_the_largest_unit_that_fits() {
        assert_eq!(span(30), "1 minute");
        assert_eq!(span(2_400), "40 minutes");
        assert_eq!(span(5 * 3_600), "5 hours");
        assert_eq!(span(9 * DAY), "9 days");
    }
}
