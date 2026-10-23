//! Launch days and hackathons (proposal 13 of
//! `docs/social-for-builders-ux.md`): `/social/events`, every NIP-52
//! calendar event on this relay by phase, and `/social/ev/<npub>/<d>` (or an
//! `naddr`), one event: when and where, its hosts, RSVPs counted by person,
//! a live stream, its room, its badges and the gallery of what was shipped
//! for it. The feed's sidebar shows what is coming up.
//!
//! A submission is this page's convention, not NIP-52's (NIP-52 links a
//! calendar event to calendars, not projects to events): a repo (kind 30617)
//! or a ship post (a kind 1 marked `t` `ship` or carrying `imeta` media, or
//! a kind 31990 app event) that `a`-tags the calendar event. A ship post by
//! a repo's maker that also tags the repo joins that repo's entry. Once the
//! event has ended, a submission whose author holds one of the event's
//! badges (see [`crate::badges`]) is pinned first.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use nostr::event::{Event, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use nostr::nips::nip19::{FromBech32, Nip19Coordinate};
use nostr::types::Timestamp;
use relay_ui::chrome::{self, Profiles, avatar, escape, icon, name, npub, plain_name, svg};
use ui_nip34::card::plural;
use ui_nip34::repo::{ANNOUNCEMENT, RepoAddress};

use crate::badges::{self, AWARD, Badge, DEFINITION};
use crate::calendar::{self, CalendarEvent, DATE_EVENT, Phase, RSVP, Rsvps, TIME_EVENT};
use crate::kinds::{APP_HANDLER, NOTE, REACTION};
use crate::live::{self, LIVE_ACTIVITY};
use crate::media::{self, Media};
use crate::{PREFIX, Pages, content, html, load_cards, not_found, parse_pubkey, post, rooms, ship};

/// The most calendar events a page reads.
const CALENDAR_QUERY: usize = 500;
/// The most past events the index lists.
const PAST_SHOWN: usize = 20;
/// The most events the feed's "Coming up" box lists.
const COMING_UP: usize = 3;
/// The most submissions, RSVPs or awards one query reads.
const LIMIT: usize = 2_000;

/// The calendar events on this relay, newest of each.
async fn calendar(pages: &Pages) -> Vec<CalendarEvent> {
    let events = pages
        .store
        .query(
            Filter::new()
                .kinds([Kind::from(DATE_EVENT), Kind::from(TIME_EVENT)])
                .limit(CALENDAR_QUERY),
        )
        .await;
    calendar::newest_each(&events)
}

/// RSVP counts for each of `events`, by coordinate.
async fn rsvp_counts(pages: &Pages, events: &[&CalendarEvent]) -> HashMap<String, Rsvps> {
    if events.is_empty() {
        return HashMap::new();
    }
    let coordinates: Vec<String> = events.iter().map(|event| event.coordinate()).collect();
    let answers = pages
        .store
        .query(
            Filter::new()
                .kind(Kind::from(RSVP))
                .custom_tags(SingleLetterTag::LOWERCASE_A, coordinates.clone())
                .limit(LIMIT),
        )
        .await;
    coordinates
        .into_iter()
        .map(|coordinate| {
            let counts = calendar::rsvps(&answers, &coordinate);
            (coordinate, counts)
        })
        .collect()
}

/// `MON` over the day: a calendar page's corner.
fn date_badge(event: &CalendarEvent) -> String {
    let (month, day) = event.badge_date();
    format!(
        "<span class=\"date-badge\" aria-hidden=\"true\"><span class=\"month\">{}</span>\
         <span class=\"day\">{}</span></span>",
        escape(&month),
        escape(&day)
    )
}

/// The phase chip: how long until it starts, how long it has left, or that
/// it is over.
fn phase_chip(event: &CalendarEvent, now: i64, winners: bool) -> String {
    match event.phase(now) {
        Phase::Before => format!(
            "<span class=\"chip\" data-phase=\"before\">Starts in {}</span>",
            calendar::span(event.starts_at() - now)
        ),
        Phase::Live => format!(
            "<span class=\"chip chip-live\" data-phase=\"live\">Live now &middot; {} left</span>",
            calendar::span(event.ends_at() - now)
        ),
        Phase::Ended => format!(
            "<span class=\"chip\" data-phase=\"ended\">Ended{}</span>",
            if winners {
                " &middot; winners announced"
            } else {
                ""
            }
        ),
    }
}

/// `N going · M interested`: distinct people by their newest RSVP.
fn going(counts: Rsvps) -> String {
    format!(
        "{} going &middot; {} interested",
        counts.accepted, counts.tentative
    )
}

/// One row of the index.
fn event_row(event: &CalendarEvent, counts: Rsvps, now: i64) -> String {
    let location = event
        .locations()
        .first()
        .map(|place| format!(" &middot; {}", escape(place)))
        .unwrap_or_default();
    format!(
        "<li class=\"event-row\" data-event=\"{}\">{}<div class=\"grow\"><div class=\"line\">\
         <a class=\"strong\" href=\"{}\">{}</a>{}</div><div class=\"muted small\">{}{location}</div>\
         <div class=\"muted small\">{}</div></div></li>",
        escape(&event.coordinate()),
        date_badge(event),
        escape(&event.path()),
        escape(event.title()),
        phase_chip(event, now, false),
        escape(&event.time_label()),
        going(counts),
    )
}

/// `/social/events`: live, upcoming and past calendar events.
pub(crate) async fn index(State(pages): State<Pages>) -> Response {
    let now = chrome::now();
    let (live, upcoming, mut past) = calendar::schedule(calendar(&pages).await, now);
    past.truncate(PAST_SHOWN);
    let all: Vec<&CalendarEvent> = live.iter().chain(&upcoming).chain(&past).collect();
    let counts = rsvp_counts(&pages, &all).await;
    let mut body = String::from(
        "<h1 class=\"page-title\">Events</h1><p class=\"muted small\">Launch days, hackathons and \
         meetups: NIP-52 calendar events (kinds 31922 and 31923) written to this relay. Times are \
         in UTC. RSVPs count people, by each one's newest answer; an RSVP is one paid write from a \
         client.</p>",
    );
    for (title, events, empty) in [
        ("Live now", &live, ""),
        ("Upcoming", &upcoming, "Nothing is scheduled."),
        ("Past", &past, "Nothing has happened here yet."),
    ] {
        if events.is_empty() && empty.is_empty() {
            continue;
        }
        body.push_str(&format!(
            "<section class=\"box\" data-section=\"{}\"><div class=\"box-header\"><span class=\"strong\">{title}</span></div>",
            title.to_ascii_lowercase().replace(' ', "-")
        ));
        if events.is_empty() {
            body.push_str(&format!("<div class=\"empty\">{empty}</div>"));
        } else {
            body.push_str("<ul class=\"rows\">");
            for event in events {
                let count = counts.get(&event.coordinate()).copied().unwrap_or_default();
                body.push_str(&event_row(event, count, now));
            }
            body.push_str("</ul>");
        }
        body.push_str("</section>");
    }
    html(&pages, StatusCode::OK, "Events", None, &body)
}

/// The feed sidebar's "Coming up": the next live or upcoming events.
pub(crate) async fn coming_up(pages: &Pages) -> String {
    let now = chrome::now();
    let (live, upcoming, _) = calendar::schedule(calendar(pages).await, now);
    let next: Vec<&CalendarEvent> = live.iter().chain(&upcoming).take(COMING_UP).collect();
    let counts = rsvp_counts(pages, &next).await;
    let mut out = String::from("<div class=\"side-card\" data-coming-up><h2>Coming up</h2>");
    if next.is_empty() {
        out.push_str("<p class=\"muted small\">Nothing is scheduled.</p>");
    } else {
        out.push_str("<ul class=\"coming-up\">");
        for event in next {
            let count = counts.get(&event.coordinate()).copied().unwrap_or_default();
            out.push_str(&format!(
                "<li><a class=\"coming\" href=\"{}\">{}<span class=\"who\"><span class=\"strong\">{}</span>\
                 <span class=\"muted small\">{} going{}</span></span></a></li>",
                escape(&event.path()),
                date_badge(event),
                escape(event.title()),
                count.accepted,
                if event.phase(now) == Phase::Live {
                    " &middot; live now"
                } else {
                    ""
                },
            ));
        }
        out.push_str("</ul>");
    }
    out.push_str(&format!(
        "<a class=\"small\" href=\"{PREFIX}/events\">All events</a></div>"
    ));
    out
}

/// `/social/ev/<naddr>`: a redirect to the event's page.
pub(crate) async fn redirect(State(pages): State<Pages>, Path(target): Path<String>) -> Response {
    let text = target.strip_prefix("nostr:").unwrap_or(&target);
    let Ok(address) = Nip19Coordinate::from_bech32(text) else {
        return not_found(&pages);
    };
    let coordinate = address.coordinate;
    if !matches!(coordinate.kind.as_u16(), DATE_EVENT | TIME_EVENT)
        || coordinate.identifier.is_empty()
    {
        return not_found(&pages);
    }
    (
        StatusCode::MOVED_PERMANENTLY,
        [(
            header::LOCATION,
            format!(
                "{PREFIX}/ev/{}/{}",
                npub(&coordinate.public_key),
                chrome::percent_encode(&coordinate.identifier)
            ),
        )],
    )
        .into_response()
}

/// One entry of an event's gallery.
#[derive(Debug, Clone)]
struct Submission {
    /// What `data-id` names: a repo's coordinate or a post's id.
    id: String,
    title: String,
    description: Option<String>,
    author: PublicKey,
    href: String,
    media: Option<Media>,
    stat: Option<String>,
    at: Timestamp,
}

/// `submissions` in gallery order: once the event has `ended`, those whose
/// author is in `winners` first; newest first within each. A pure function.
fn rank(submissions: &mut [Submission], winners: &HashSet<PublicKey>, ended: bool) {
    submissions.sort_by(|a, b| {
        let pinned = |s: &Submission| ended && winners.contains(&s.author);
        pinned(b)
            .cmp(&pinned(a))
            .then_with(|| b.at.cmp(&a.at))
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// Whether `event` names `coordinate` in an `a` tag.
fn a_tags(event: &Event, coordinate: &str) -> bool {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .any(|tag| matches!(tag, [name, value, ..] if name == "a" && value == coordinate))
}

/// Whether a kind 1 is marked as a launch: `t` `ship` or `imeta` media.
fn marked_ship(event: &Event) -> bool {
    event.tags.iter().map(|tag| tag.as_slice()).any(
        |tag| matches!(tag, [name, value, ..] if name == "t" && value.eq_ignore_ascii_case("ship")),
    ) || !media::imeta(event).is_empty()
}

/// The first image a post shows (a video's poster is not an image here).
fn first_image(event: &Event) -> Option<Media> {
    post::launch_media(event)
        .into_iter()
        .find(|media| !media.is_video())
}

/// The gallery: what `a`-tags the event at `coordinate`.
async fn submissions(pages: &Pages, coordinate: &str) -> Vec<Submission> {
    let store = &pages.store;
    let tagged: Vec<Event> = store
        .query(
            Filter::new()
                .kinds([
                    Kind::from(ANNOUNCEMENT),
                    Kind::from(NOTE),
                    Kind::from(APP_HANDLER),
                ])
                .custom_tag(SingleLetterTag::LOWERCASE_A, coordinate.to_owned())
                .limit(LIMIT),
        )
        .await
        .into_iter()
        .filter(|event| a_tags(event, coordinate))
        .collect();
    let mut repos: Vec<RepoAddress> = Vec::new();
    for event in &tagged {
        let named = match event.kind.as_u16() {
            ANNOUNCEMENT => RepoAddress::of(event).into_iter().collect(),
            _ => RepoAddress::tagged(event, "a"),
        };
        for repo in named {
            if !repos.contains(&repo) {
                repos.push(repo);
            }
        }
    }
    let cards = load_cards(store, repos).await;
    let links = pages.links();
    let repo_entry = |repo: &RepoAddress, at: Timestamp| {
        cards.get(repo).map(|card| Submission {
            id: repo.coordinate(),
            title: card.name().to_owned(),
            description: card.description().map(str::to_owned),
            author: *repo.owner(),
            href: links
                .social_path(repo)
                .or_else(|| links.repo_path(repo))
                .unwrap_or_else(|| format!("{PREFIX}/r/{}", repo.path_segments())),
            media: None,
            stat: Some(format!(
                "{} {}",
                card.stars(),
                plural(card.stars(), "star", "stars")
            )),
            at,
        })
    };
    let mut entries: Vec<Submission> = Vec::new();
    let mut posts: Vec<&Event> = Vec::new();
    for event in &tagged {
        if event.kind.as_u16() == ANNOUNCEMENT {
            if let Some(repo) = RepoAddress::of(event)
                && let Some(entry) = repo_entry(&repo, event.created_at)
                && !entries.iter().any(|held| held.id == entry.id)
            {
                entries.push(entry);
            }
        } else if (event.kind.as_u16() == NOTE && marked_ship(event))
            || ship::app_site(event).is_some()
            || (event.kind.as_u16() == APP_HANDLER && ship::app_metadata(event).0.is_some())
        {
            posts.push(event);
        }
    }
    posts.sort_by(|a, b| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
    let mut standalone: Vec<&Event> = Vec::new();
    for event in posts {
        let made = RepoAddress::tagged(event, "a").into_iter().find(|repo| {
            cards
                .get(repo)
                .is_some_and(|card| card.is_maker(&event.pubkey))
        });
        let Some(repo) = made else {
            standalone.push(event);
            continue;
        };
        let id = repo.coordinate();
        if !entries.iter().any(|held| held.id == id)
            && let Some(entry) = repo_entry(&repo, event.created_at)
        {
            entries.push(entry);
        }
        if let Some(entry) = entries.iter_mut().find(|held| held.id == id) {
            if entry.media.is_none() {
                entry.media = first_image(event);
            }
            entry.at = entry.at.max(event.created_at);
        }
    }
    let reactions: Vec<Event> = if standalone.is_empty() {
        Vec::new()
    } else {
        store
            .query(
                Filter::new()
                    .kind(Kind::from(REACTION))
                    .events(standalone.iter().map(|event| event.id))
                    .limit(LIMIT),
            )
            .await
    };
    for event in standalone {
        let id = event.id.to_hex();
        let people = reactions
            .iter()
            .filter(|reaction| {
                reaction
                    .tags
                    .iter()
                    .map(|tag| tag.as_slice())
                    .any(|tag| matches!(tag, [name, value, ..] if name == "e" && *value == id))
            })
            .map(|reaction| reaction.pubkey)
            .collect::<HashSet<_>>()
            .len();
        let (title, description) = if event.kind.as_u16() == APP_HANDLER {
            let (name, about) = ship::app_metadata(event);
            (name.unwrap_or_else(|| "An app".to_owned()), about)
        } else {
            let title = content::headline(&event.content, 80);
            (
                if title.is_empty() {
                    "A launch".to_owned()
                } else {
                    title
                },
                None,
            )
        };
        entries.push(Submission {
            id,
            title,
            description,
            author: event.pubkey,
            href: post::event_path(event),
            media: first_image(event),
            stat: Some(format!(
                "{people} {} reacted",
                plural(people, "person", "people")
            )),
            at: event.created_at,
        });
    }
    entries
}

/// A gallery card.
fn submission_card(
    submission: &Submission,
    names: &HashMap<PublicKey, String>,
    prizes: &[&Badge],
) -> String {
    let media = submission
        .media
        .as_ref()
        .map(|media| media.html(""))
        .unwrap_or_else(|| "<span class=\"muted small\">No picture</span>".to_owned());
    let chips: String = prizes
        .iter()
        .map(|badge| {
            format!(
                "<span class=\"chip chip-win\">{}</span>",
                escape(badge.name())
            )
        })
        .collect();
    let description = submission
        .description
        .as_deref()
        .map(|text| {
            format!(
                "<span class=\"muted\">{}</span>",
                escape(&content::clip(text, 140))
            )
        })
        .unwrap_or_default();
    let stat = submission
        .stat
        .as_deref()
        .map(|stat| format!(" &middot; {stat}"))
        .unwrap_or_default();
    format!(
        "<a class=\"sub-card\" href=\"{}\" data-submission data-id=\"{}\"{}><div class=\"sub-media\">{media}</div>\
         <div class=\"sub-body\"><div class=\"line\"><span class=\"strong\">{}</span>{chips}</div>{description}\
         <span class=\"muted small\">by {}{stat}</span></div></a>",
        escape(&submission.href),
        escape(&submission.id),
        if prizes.is_empty() {
            ""
        } else {
            " data-winner"
        },
        escape(&submission.title),
        escape(&plain_name(names, &submission.author)),
    )
}

/// The event's hero: what, when, where, who, and RSVP.
fn hero(
    event: &CalendarEvent,
    names: &HashMap<PublicKey, String>,
    counts: Rsvps,
    now: i64,
    winners: bool,
) -> String {
    let hosts = event.hosts();
    let host_names: Vec<String> = hosts
        .iter()
        .map(|key| {
            format!(
                "<a href=\"{PREFIX}/following/{}\">{}</a>",
                npub(key),
                escape(&plain_name(names, key))
            )
        })
        .collect();
    let hosted = match host_names.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} and {last}", rest.join(", ")),
        None => String::new(),
    };
    let places: Vec<String> = event
        .locations()
        .iter()
        .map(|place| {
            if place.starts_with("https://") {
                format!(
                    "<a href=\"{0}\" rel=\"nofollow noopener noreferrer\">{0}</a>",
                    escape(place)
                )
            } else {
                escape(place)
            }
        })
        .collect();
    let mut facts = format!(
        "<p class=\"event-when\">{}{}</p>",
        svg(icon::CALENDAR, "icon-sm"),
        escape(&event.time_label())
    );
    if !places.is_empty() {
        facts.push_str(&format!(
            "<p class=\"event-where\">{}{}</p>",
            svg(icon::LOCATION, "icon-sm"),
            places.join(" &middot; ")
        ));
    }
    facts.push_str(&format!("<p class=\"event-hosts\">Hosted by {hosted}</p>"));
    let summary = event
        .summary()
        .map(|summary| format!("<p class=\"event-summary\">{}</p>", escape(&summary)))
        .unwrap_or_default();
    let topics: String = event
        .topics()
        .iter()
        .map(|topic| format!("<span class=\"chip chip-topic\">{}</span>", escape(topic)))
        .collect();
    format!(
        "<section class=\"event-hero\" data-phase=\"{}\"><div class=\"event-hero-main\">\
         <div class=\"chips\"><span class=\"chip\">Calendar event</span>{}{topics}</div>\
         <h1 class=\"event-title\">{}</h1>{facts}{summary}</div>\
         <div class=\"event-hero-side\"><span class=\"small\" data-rsvps>{} (RSVPs, counted by person)</span>\
         <a class=\"btn btn-success btn-lg\" href=\"nostr:{}\">RSVP in your client</a>\
         <span class=\"small\">An RSVP is a kind 31925 your client writes; one paid write.</span></div></section>",
        match event.phase(now) {
            Phase::Before => "before",
            Phase::Live => "live",
            Phase::Ended => "ended",
        },
        phase_chip(event, now, winners),
        escape(event.title()),
        going(counts),
        escape(&event.naddr()),
    )
}

/// The badges card.
fn badges_card(badges: &[Badge], counts: &[usize]) -> String {
    let mut out = String::from("<div class=\"side-card\" data-badges><h2>Badges</h2>");
    if badges.is_empty() {
        out.push_str(
            "<p class=\"muted small\">The hosts have defined no badges for this event.</p></div>",
        );
        return out;
    }
    out.push_str("<ul class=\"badges\">");
    for (badge, count) in badges.iter().zip(counts) {
        let picture = badge.image_html().unwrap_or_else(|| {
            format!(
                "<span class=\"badge-disc\" aria-hidden=\"true\">{}</span>",
                escape(&content::clip(badge.name(), 12))
            )
        });
        let description = badge
            .description()
            .map(|text| format!("<span class=\"muted small\">{}</span>", escape(text)))
            .unwrap_or_default();
        out.push_str(&format!(
            "<li data-badge=\"{}\">{picture}<span class=\"who\"><span class=\"strong\">{}</span>{description}\
             <span class=\"muted small\">awarded to {count} {}</span></span></li>",
            escape(&badge.coordinate()),
            escape(badge.name()),
            plural(*count, "person", "people"),
        ));
    }
    out.push_str(
        "</ul><p class=\"muted small\">NIP-58 badges a host defined for this event. An award counts \
         only when the host who defined the badge signed it.</p></div>",
    );
    out
}

/// `/social/ev/<npub>/<d>`: one calendar event.
pub(crate) async fn page(
    State(pages): State<Pages>,
    Path((author, d)): Path<(String, String)>,
) -> Response {
    let Some(author) = parse_pubkey(&author) else {
        return not_found(&pages);
    };
    let store = &pages.store;
    let found = store
        .query(
            Filter::new()
                .kinds([Kind::from(DATE_EVENT), Kind::from(TIME_EVENT)])
                .author(author)
                .identifier(d.clone())
                .limit(20),
        )
        .await;
    let mut found = calendar::newest_each(&found);
    found.retain(|event| event.d() == d && *event.author() == author);
    found.sort_by_key(|event| std::cmp::Reverse(event.event().created_at));
    let Some(event) = found.into_iter().next() else {
        return not_found(&pages);
    };
    let now = chrome::now();
    let coordinate = event.coordinate();
    let hosts = event.hosts();

    let answers = store
        .query(
            Filter::new()
                .kind(Kind::from(RSVP))
                .custom_tag(SingleLetterTag::LOWERCASE_A, coordinate.clone())
                .limit(LIMIT),
        )
        .await;
    let counts = calendar::rsvps(&answers, &coordinate);
    let mut gallery = submissions(&pages, &coordinate).await;
    let definitions = store
        .query(
            Filter::new()
                .kind(Kind::from(DEFINITION))
                .authors(hosts.clone())
                .limit(500),
        )
        .await;
    let badges = badges::event_badges(&definitions, event.event(), &coordinate, &hosts);
    let holders = if badges.is_empty() {
        HashMap::new()
    } else {
        let awards = store
            .query(
                Filter::new()
                    .kind(Kind::from(AWARD))
                    .authors(hosts.clone())
                    .custom_tags(
                        SingleLetterTag::LOWERCASE_A,
                        badges.iter().map(Badge::coordinate),
                    )
                    .limit(LIMIT),
            )
            .await;
        badges::holders(&awards, &badges)
    };
    let badge_counts = badges::counts(&holders, badges.len());
    let ended = event.phase(now) == Phase::Ended;
    let winners: HashSet<PublicKey> = gallery
        .iter()
        .map(|entry| entry.author)
        .filter(|author| holders.contains_key(author))
        .collect();
    rank(&mut gallery, &winners, ended);
    let streams = store
        .query(
            Filter::new()
                .kind(Kind::from(LIVE_ACTIVITY))
                .custom_tag(SingleLetterTag::LOWERCASE_A, coordinate.clone())
                .limit(20),
        )
        .await;

    let mut keys: Vec<PublicKey> = hosts.clone();
    for key in event
        .participants()
        .into_iter()
        .map(|(key, _)| key)
        .chain(gallery.iter().map(|entry| entry.author))
        .chain(content::mentioned_keys(&event.event().content))
    {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let profiles = Profiles::load(store, &keys).await;
    let names = profiles.names();

    let mut body = hero(&event, names, counts, now, ended && !winners.is_empty());
    if let Some(image) = media::image_tag(event.event()) {
        body.push_str(&image.html("hero"));
    }
    if let Some(banner) = live::banner(&streams, now) {
        body.push_str(&banner);
    }
    body.push_str("<div class=\"split\"><div class=\"main\">");
    let about = content::render(&event.event().content, &[], names);
    if !about.html.trim().is_empty() {
        body.push_str(&format!(
            "<section class=\"box\"><div class=\"box-header\"><span class=\"strong\">About</span></div>\
             <div class=\"box-body\"><p class=\"text\">{}</p></div></section>",
            about.html.trim()
        ));
    }
    let (title, note) = match event.phase(now) {
        Phase::Ended => (
            format!(
                "{} {} shipped",
                gallery.len(),
                plural(gallery.len(), "project", "projects")
            ),
            if winners.is_empty() {
                "Newest first"
            } else {
                "Winners pinned first"
            },
        ),
        _ => (
            "Submitted so far".to_owned(),
            "Newest first; ranking opens at the end",
        ),
    };
    body.push_str(&format!(
        "<section data-gallery><div class=\"toolbar\"><h2 class=\"section-title\">{}</h2>\
         <span class=\"muted small\">{note}</span></div>",
        escape(&title)
    ));
    if event.phase(now) == Phase::Before {
        body.push_str(&format!(
            "<div class=\"box\"><div class=\"empty\"><h2>Submissions open when the event starts</h2>\
             <p>The gallery shows every repo or ship post that links to this event{}.</p></div></div>",
            if gallery.is_empty() {
                String::new()
            } else {
                format!(
                    "; {} {} so far",
                    gallery.len(),
                    plural(gallery.len(), "submission", "submissions")
                )
            }
        ));
    } else if gallery.is_empty() {
        body.push_str(
            "<div class=\"box\"><div class=\"empty\">Nothing has been submitted yet.</div></div>",
        );
    } else {
        body.push_str("<div class=\"gallery\">");
        for entry in &gallery {
            let prizes: Vec<&Badge> = holders
                .get(&entry.author)
                .map(|held| held.iter().filter_map(|index| badges.get(*index)).collect())
                .unwrap_or_default();
            body.push_str(&submission_card(entry, names, &prizes));
        }
        body.push_str("</div>");
    }
    body.push_str(&format!(
        "<p class=\"muted small\">A submission is a repo (kind 30617) or a ship post (a kind 1 tagged \
         <code>ship</code> or with media, or a kind 31990 app) that names this event in an <code>a</code> \
         tag: <code>{}</code>. This is a convention of this relay's pages; NIP-52 itself links events to \
         calendars, not projects to events.</p></section>",
        escape(&coordinate)
    ));
    body.push_str("</div><aside class=\"sidebar\">");
    if let (Some(groups), Some(id)) = (pages.groups.as_ref(), event.room())
        && let Some(room) = groups.room(id).await
    {
        body.push_str(&rooms::room_card(&pages, &room).await);
    }
    body.push_str(&badges_card(&badges, &badge_counts));
    let participants = event.participants();
    if !participants.is_empty() {
        body.push_str("<div><h3>People</h3><ul>");
        for (key, role) in &participants {
            body.push_str(&format!(
                "<li><a href=\"{PREFIX}/following/{}\">{} {}</a>{}</li>",
                npub(key),
                avatar(&plain_name(names, key), true),
                name(names, key),
                role.as_deref()
                    .map(|role| format!(" <span class=\"muted small\">{}</span>", escape(role)))
                    .unwrap_or_default(),
            ));
        }
        body.push_str("</ul></div>");
    }
    body.push_str(&format!(
        "<div><a href=\"{PREFIX}/events\">All events</a></div></aside></div>"
    ));
    html(&pages, StatusCode::OK, event.title(), None, &body)
}

#[cfg(test)]
mod tests {
    use nostr::key::Keys;

    use super::*;

    fn entry(author: PublicKey, at: u64, id: &str) -> Submission {
        Submission {
            id: id.to_owned(),
            title: id.to_owned(),
            description: None,
            author,
            href: String::new(),
            media: None,
            stat: None,
            at: Timestamp::from(at),
        }
    }

    #[test]
    fn winners_are_pinned_first_once_the_event_has_ended() {
        let (winner, other) = (Keys::generate().public_key(), Keys::generate().public_key());
        let mut gallery = vec![
            entry(other, 3, "newest"),
            entry(winner, 1, "won"),
            entry(other, 2, "middle"),
        ];
        let winners = HashSet::from([winner]);
        rank(&mut gallery, &winners, false);
        let order: Vec<&str> = gallery.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(
            order,
            ["newest", "middle", "won"],
            "before the end: newest first"
        );
        rank(&mut gallery, &winners, true);
        let order: Vec<&str> = gallery.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(order, ["won", "newest", "middle"]);
    }
}
