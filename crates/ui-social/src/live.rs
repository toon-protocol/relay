//! NIP-53 live activities (kind 30311) beside an event or a room: a banner
//! that says a stream is live, how many are in it, and where to watch. The
//! stream is linked, never embedded: its `streaming` URL (`https` only) is
//! a plain link the visitor follows.
//!
//! NIP-53 has no tag for "this stream belongs to that calendar event or
//! group", so the link is this page's convention (`docs/apps/social.md`): a
//! 30311 that `a`-tags a calendar event is that event's, and one with an
//! `h` tag is that room's. NIP-53 lets a client treat a `live` activity not
//! updated for an hour as ended, and this page does.

use nostr::event::{Event, Kind};
use relay_ui::chrome::{escape, icon, svg};
use ui_nip34::repo::{first_tag, newest};

/// NIP-53 live streaming event.
pub(crate) const LIVE_ACTIVITY: u16 = 30_311;

/// How long a `live` activity stays live without an update.
const STALE: i64 = 3_600;

/// A live activity's state, as this page reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Status {
    Planned,
    Live,
    Ended,
}

/// A live activity, read.
#[derive(Debug, Clone)]
pub(crate) struct Live {
    title: String,
    status: Status,
    participants: usize,
    streaming: Option<String>,
}

impl Live {
    /// `event` as a live activity at `now`. A `live` status left unupdated
    /// for an hour reads as ended. A pure function.
    pub(crate) fn read(event: &Event, now: i64) -> Option<Self> {
        if event.kind != Kind::from(LIVE_ACTIVITY) {
            return None;
        }
        let updated = i64::try_from(event.created_at.as_secs()).unwrap_or(0);
        let status = match first_tag(event, "status")
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("live") if now - updated <= STALE => Status::Live,
            Some("planned") => Status::Planned,
            _ => Status::Ended,
        };
        let participants = first_tag(event, "current_participants")
            .and_then(|count| count.trim().parse::<usize>().ok())
            .unwrap_or_else(|| {
                let mut keys: Vec<&str> = event
                    .tags
                    .iter()
                    .map(|tag| tag.as_slice())
                    .filter_map(|tag| match tag {
                        [name, key, ..] if name == "p" => Some(key.as_str()),
                        _ => None,
                    })
                    .collect();
                keys.sort_unstable();
                keys.dedup();
                keys.len()
            });
        let streaming = first_tag(event, "streaming")
            .map(str::trim)
            .filter(|url| {
                url.strip_prefix("https://")
                    .is_some_and(|rest| !rest.is_empty())
                    && !url
                        .chars()
                        .any(|c| c.is_whitespace() || c.is_control() || "\"<>".contains(c))
            })
            .map(str::to_owned);
        Some(Self {
            title: first_tag(event, "title")
                .map(str::trim)
                .filter(|title| !title.is_empty())
                .unwrap_or("Live stream")
                .to_owned(),
            status,
            participants,
            streaming,
        })
    }

    pub(crate) fn status(&self) -> Status {
        self.status
    }

    /// The banner: live or planned, its title, who is in it, and a link to
    /// watch. An ended activity has none.
    pub(crate) fn banner(&self) -> Option<String> {
        let (class, chip) = match self.status {
            Status::Live => ("live-banner", "Live"),
            Status::Planned => ("live-banner planned", "Planned"),
            Status::Ended => return None,
        };
        let watch = self
            .streaming
            .as_deref()
            .map(|url| {
                format!(
                    "<a class=\"btn\" href=\"{}\" rel=\"nofollow noopener noreferrer\">{}Watch</a>",
                    escape(url),
                    svg(icon::PLAY, "icon-sm")
                )
            })
            .unwrap_or_default();
        Some(format!(
            "<div class=\"{class}\" data-live><span class=\"chip chip-live\">{chip}</span>\
             <span class=\"grow\">{} &middot; {} {}</span>{watch}</div>",
            escape(&self.title),
            self.participants,
            if self.status == Status::Live {
                "watching"
            } else {
                "named"
            },
        ))
    }
}

/// The banner of the newest of `events` that is live (else planned), if
/// any. A pure function.
pub(crate) fn banner(events: &[Event], now: i64) -> Option<String> {
    let read: Vec<(&Event, Live)> = events
        .iter()
        .filter_map(|event| Live::read(event, now).map(|live| (event, live)))
        .collect();
    for wanted in [Status::Live, Status::Planned] {
        let newest_event = newest(
            read.iter()
                .filter(|(_, live)| live.status() == wanted)
                .map(|(event, _)| *event),
        );
        if let Some(event) = newest_event {
            return Live::read(event, now).and_then(|live| live.banner());
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;
    use nostr::types::Timestamp;

    use super::*;

    fn activity(at: u64, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(LIVE_ACTIVITY), "")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .custom_created_at(Timestamp::from(at))
            .finalize(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn a_live_activity_goes_stale_after_an_hour_without_an_update() {
        let event = activity(10_000, &[&["d", "x"], &["status", "live"]]);
        assert_eq!(
            Live::read(&event, 10_000 + 3_600).unwrap().status(),
            Status::Live
        );
        assert_eq!(
            Live::read(&event, 10_000 + 3_601).unwrap().status(),
            Status::Ended
        );
        assert!(banner(std::slice::from_ref(&event), 20_000).is_none());
    }

    #[test]
    fn the_banner_counts_participants_and_links_only_an_https_stream() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        let event = activity(
            100,
            &[
                &["d", "x"],
                &["title", "Kickoff <b>"],
                &["status", "live"],
                &["streaming", "https://stream.test/live.m3u8"],
                &["p", &a, "", "Host"],
                &["p", &b, "", "Speaker"],
                &["p", &b, "", "Speaker"],
            ],
        );
        let html = banner(std::slice::from_ref(&event), 200).unwrap();
        assert!(html.contains("Kickoff &lt;b&gt; &middot; 2 watching"));
        assert!(html.contains("href=\"https://stream.test/live.m3u8\""));

        let counted = activity(
            100,
            &[
                &["status", "live"],
                &["current_participants", "36"],
                &["streaming", "javascript:alert(1)"],
            ],
        );
        let html = banner(std::slice::from_ref(&counted), 200).unwrap();
        assert!(html.contains("36 watching"));
        assert!(
            !html.contains("href"),
            "a stream that is not https is not linked"
        );
    }
}
