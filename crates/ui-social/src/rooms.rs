//! Rooms: NIP-29 groups on this relay, read without signing in
//! (proposal 14 of `docs/social-for-builders-ux.md`). `/social/g/<id>` is a
//! room's read-only view: its NIP-C7 chat (kind 9) oldest to newest, its
//! threads (kind 11), and a live stream when one is on (NIP-53).
//!
//! A group's metadata (kind 39000) counts only when the relay's own key
//! signed it, as NIP-29 says (and as `crates/relay/src/groups.rs` writes
//! it), so a client cannot dress a room up. What a visitor may read follows
//! the relay's own read rules for a connection that has not authenticated
//! (NIP-42), since a server-rendered page cannot authenticate for its
//! visitor:
//!
//! - a `private` (or `hidden`) group shows nothing, not even its name;
//! - a `closed` group shows its name and what it is about, and none of its
//!   events (this relay restricts a closed group's reads, which is stricter
//!   than upstream NIP-29, where `closed` only refuses join requests);
//! - anything else is readable.
//!
//! The relay applies that rule to the store it hands every app UI, so a note
//! posted into a private group never reaches the feed, a repo page or
//! `/social/e/<id>` (nor `/repo`), and a private group's metadata is not
//! there to read: its room is not found, like a room that does not exist.
//!
//! A repo's room is a convention (`docs/apps/social.md`): the group whose
//! id is the repo's `d` and whose admins (kind 39001, relay-signed) include
//! an authorized author of the repo (ADR 0002). Neither NIP-29 nor NIP-34
//! has a tag that links the two, and this relay's group metadata carries
//! only the fields NIP-29 names.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use nostr::event::{Event, EventId, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use nostr::nips::nip01::Coordinate;
use nostr::nips::nip19::{Nip19Coordinate, ToBech32};
use nostr::types::Timestamp;
use relay_ui::chrome::{self, Profiles, ago, avatar, escape, icon, name, plain_name, svg};
use relay_ui::ReadOnlyStore;
use ui_nip34::card::{RepoCard, plural};
use ui_nip34::repo::{d_tag, first_tag, newest};

use crate::kinds::THREAD;
use crate::live::{self, LIVE_ACTIVITY};
use crate::media::Media;
use crate::{PREFIX, Pages, content, html};

/// NIP-C7 chat message.
pub(crate) const CHAT: u16 = 9;
/// NIP-29 group metadata, signed by the relay.
pub(crate) const GROUP_METADATA: u16 = 39_000;
/// NIP-29 group admins, signed by the relay.
pub(crate) const GROUP_ADMINS: u16 = 39_001;
/// NIP-29 group members, signed by the relay.
pub(crate) const GROUP_MEMBERS: u16 = 39_002;

/// The most chat messages a room shows: the newest, oldest first.
const TRANSCRIPT: usize = 100;
/// The most threads a room lists.
const THREADS: usize = 20;
/// The messages a room's card shows elsewhere.
const PREVIEW: usize = 5;

/// What `/social` knows of this relay's groups: the key that signs their
/// metadata, and the store as the relay serves it to a visitor.
#[derive(Clone)]
pub(crate) struct Groups {
    signer: PublicKey,
    raw: ReadOnlyStore,
}

impl Groups {
    pub(crate) fn new(signer: PublicKey, store: ReadOnlyStore) -> Self {
        Self { signer, raw: store }
    }

    /// The room `id`, if the relay has signed metadata for it.
    pub(crate) async fn room(&self, id: &str) -> Option<Room> {
        rooms(&self.raw, self.signer, [id.to_owned()])
            .await
            .remove(id)
    }

    /// The room a repo keeps: the group named the repo's `d`, readable by
    /// anyone, that an authorized author of the repo administers.
    pub(crate) async fn repo_room(&self, card: &RepoCard) -> Option<Room> {
        let room = self.room(card.address().d()).await?;
        if !room.readable() {
            return None;
        }
        let admins = self
            .raw
            .query(
                Filter::new()
                    .kind(Kind::from(GROUP_ADMINS))
                    .author(self.signer)
                    .identifier(room.id.clone()),
            )
            .await;
        let admins = newest(admins.iter().filter(|event| d_tag(event) == room.id))?;
        listed_keys(admins)
            .iter()
            .any(|key| card.is_maker(key))
            .then_some(room)
    }
}

/// A group, as its relay-signed metadata describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Room {
    id: String,
    name: String,
    about: Option<String>,
    picture: Option<String>,
    private: bool,
    closed: bool,
    hidden: bool,
}

impl Room {
    /// `event` as a room, if it is a 39000 that `signer` signed. A pure
    /// function.
    pub(crate) fn read(event: &Event, signer: &PublicKey) -> Option<Self> {
        if event.kind != Kind::from(GROUP_METADATA) || event.pubkey != *signer {
            return None;
        }
        let id = d_tag(event);
        if id.is_empty() {
            return None;
        }
        let flag = |name: &str| event.tags.iter().any(|tag| tag.kind() == name);
        let text = |name: &str| {
            first_tag(event, name)
                .map(str::trim)
                .filter(|text| !text.is_empty())
                .map(str::to_owned)
        };
        Some(Self {
            id: id.to_owned(),
            name: text("name").unwrap_or_else(|| id.to_owned()),
            about: text("about"),
            picture: text("picture"),
            private: flag("private"),
            closed: flag("closed"),
            hidden: flag("hidden"),
        })
    }

    /// Whether a visitor who has not signed in may read its events.
    pub(crate) fn readable(&self) -> bool {
        !self.private && !self.closed && !self.hidden
    }

    /// Whether a visitor who has not signed in may read its metadata.
    pub(crate) fn listed(&self) -> bool {
        !self.private && !self.hidden
    }

    /// The room's page.
    pub(crate) fn path(&self) -> String {
        format!("{PREFIX}/g/{}", chrome::percent_encode(&self.id))
    }

    /// The `naddr` of its metadata event, for a `nostr:` link a client
    /// opens (NIP-29 "group referencing").
    fn naddr(&self, signer: PublicKey) -> String {
        let coordinate =
            Coordinate::new(Kind::from(GROUP_METADATA), signer).identifier(self.id.clone());
        Nip19Coordinate::new(coordinate, Vec::<nostr::types::RelayUrl>::new())
            .to_bech32()
            .unwrap_or_default()
    }
}

/// The `p` keys of an event, once each.
fn listed_keys(event: &Event) -> Vec<PublicKey> {
    let mut keys: Vec<PublicKey> = Vec::new();
    for tag in event.tags.iter().map(|tag| tag.as_slice()) {
        if let [name, key, ..] = tag
            && name == "p"
            && let Ok(key) = PublicKey::from_hex(key)
            && !keys.contains(&key)
        {
            keys.push(key);
        }
    }
    keys
}

/// The rooms among `ids` that `signer` has signed metadata for, newest
/// metadata each.
async fn rooms(
    store: &ReadOnlyStore,
    signer: PublicKey,
    ids: impl IntoIterator<Item = String>,
) -> HashMap<String, Room> {
    let ids: Vec<String> = ids.into_iter().collect();
    if ids.is_empty() {
        return HashMap::new();
    }
    let events = store
        .query(
            Filter::new()
                .kind(Kind::from(GROUP_METADATA))
                .author(signer)
                .identifiers(ids.clone()),
        )
        .await;
    let mut found = HashMap::new();
    for id in ids {
        let held = newest(
            events
                .iter()
                .filter(|event| event.pubkey == signer && d_tag(event) == id),
        );
        if let Some(room) = held.and_then(|event| Room::read(event, &signer)) {
            found.insert(id, room);
        }
    }
    found
}

/// The newest `limit` events of `kind` in group `id`, oldest first.
async fn in_room(store: &ReadOnlyStore, id: &str, kind: u16, limit: usize) -> Vec<Event> {
    let mut events: Vec<Event> = store
        .query(
            Filter::new()
                .kind(Kind::from(kind))
                .custom_tag(SingleLetterTag::LOWERCASE_H, id.to_owned())
                .limit(limit),
        )
        .await
        .into_iter()
        .filter(|event| first_tag(event, "h") == Some(id))
        .collect();
    events.sort_by_key(|event| (event.created_at, event.id));
    events.dedup_by_key(|event| event.id);
    let skip = events.len().saturating_sub(limit);
    events.split_off(skip)
}

/// The live activities in room `id` (an `h` tag).
async fn room_streams(store: &ReadOnlyStore, id: &str) -> Vec<Event> {
    store
        .query(
            Filter::new()
                .kind(Kind::from(LIVE_ACTIVITY))
                .custom_tag(SingleLetterTag::LOWERCASE_H, id.to_owned())
                .limit(20),
        )
        .await
}

/// What stands where a chat input would.
pub(crate) const READ_ONLY_NOTE: &str = "<p class=\"muted small read-only-note\" data-read-only>\
     Read-only here; chat from your client. Each message is a paid write.</p>";

/// What a room that needs signing in shows instead of its content.
fn sign_in_box(room: Option<&Room>, signer: PublicKey) -> String {
    let link = room
        .filter(|room| room.listed())
        .map(|room| {
            format!(
                "<p><a class=\"btn\" href=\"nostr:{}\">Open in your client</a></p>",
                escape(&room.naddr(signer))
            )
        })
        .unwrap_or_default();
    format!(
        "<div class=\"box\" data-sign-in><div class=\"empty\"><h2>This room needs sign-in in a client</h2>\
         <p>Its messages are for its members. A member reads them in a Nostr client that signs in to \
         this relay (NIP-42); this page cannot sign in for you, so it shows none of them.</p>{link}</div></div>"
    )
}

/// One chat message, with the message it answers (a `q` tag) when that is
/// in view.
fn message_html(
    names: &HashMap<PublicKey, String>,
    event: &Event,
    shown: &HashMap<EventId, &Event>,
) -> String {
    let quoted = event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find_map(|tag| match tag {
            [name, id, ..] if name == "q" => EventId::from_hex(id).ok(),
            _ => None,
        })
        .and_then(|id| shown.get(&id))
        .map(|parent| {
            format!(
                "<p class=\"quote muted small\">{}{}: {}</p>",
                svg(icon::COMMENT, "icon-sm"),
                name(names, &parent.pubkey),
                escape(&content::first_line(&parent.content, 80))
            )
        })
        .unwrap_or_default();
    let text = content::render(&event.content, &[], names);
    format!(
        "<li class=\"message\" data-id=\"{}\">{}<div class=\"body\"><div class=\"line\">{} \
         <span class=\"muted small\">{}</span></div>{quoted}<p class=\"text\">{}</p></div></li>",
        event.id.to_hex(),
        avatar(&plain_name(names, &event.pubkey), true),
        name(names, &event.pubkey),
        ago(event.created_at),
        text.html.trim(),
    )
}

/// A transcript of chat messages, oldest first.
fn transcript_html(names: &HashMap<PublicKey, String>, messages: &[Event]) -> String {
    if messages.is_empty() {
        return "<p class=\"muted small\">No messages yet.</p>".to_owned();
    }
    let shown: HashMap<EventId, &Event> = messages.iter().map(|event| (event.id, event)).collect();
    let mut out = String::from("<ol class=\"chat\">");
    for message in messages {
        out.push_str(&message_html(names, message, &shown));
    }
    out.push_str("</ol>");
    out
}

/// A room's card for a sidebar: its live banner, its latest messages and a
/// link to it; or, for a room a visitor cannot read, what it needs.
pub(crate) async fn room_card(pages: &Pages, room: &Room) -> String {
    let store = &pages.store;
    let head = format!(
        "<div class=\"box-header\"><a class=\"strong\" href=\"{}\">#{}</a><span class=\"grow\"></span>\
         <span class=\"chip\">{}</span></div>",
        escape(&room.path()),
        escape(&room.name),
        if room.readable() {
            "public group"
        } else {
            "members only"
        },
    );
    if !room.readable() {
        return format!(
            "<section class=\"box room-card\" data-room=\"{}\">{head}<div class=\"box-body\">\
             <p class=\"muted small\">This room needs sign-in in a client.</p></div></section>",
            escape(&room.id)
        );
    }
    let messages = in_room(store, &room.id, CHAT, PREVIEW).await;
    let streams = room_streams(store, &room.id).await;
    let keys: Vec<PublicKey> = messages.iter().map(|event| event.pubkey).collect();
    let profiles = Profiles::load(store, &keys).await;
    let banner = live::banner(&streams, chrome::now()).unwrap_or_default();
    format!(
        "<section class=\"box room-card\" data-room=\"{}\">{head}<div class=\"box-body stack-sm\">{banner}{}\
         <a href=\"{}\">Read the room</a></div><div class=\"box-footer\">{READ_ONLY_NOTE}</div></section>",
        escape(&room.id),
        transcript_html(profiles.names(), &messages),
        escape(&room.path()),
    )
}

/// The repo page sidebar's Room box, when the repo keeps a room.
pub(crate) async fn repo_room_box(pages: &Pages, card: &RepoCard) -> String {
    let Some(groups) = pages.groups.as_ref() else {
        return String::new();
    };
    let Some(room) = groups.repo_room(card).await else {
        return String::new();
    };
    let since = Timestamp::from(
        u64::try_from(chrome::now())
            .unwrap_or(0)
            .saturating_sub(86_400),
    );
    let today = pages
        .store
        .query(
            Filter::new()
                .kind(Kind::from(CHAT))
                .custom_tag(SingleLetterTag::LOWERCASE_H, room.id.clone())
                .since(since)
                .limit(1_000),
        )
        .await
        .len();
    format!(
        "<div class=\"side-card\" data-room=\"{}\"><h2>Room</h2><p class=\"muted small\">#{}, a public \
         group on this relay (NIP-29). {today} {} today.</p><a href=\"{}\">Read the room</a></div>",
        escape(&room.id),
        escape(&room.name),
        plural(today, "message", "messages"),
        escape(&room.path()),
    )
}

/// `/social/g/<id>`: a room, read-only.
pub(crate) async fn page(State(pages): State<Pages>, Path(id): Path<String>) -> Response {
    let Some(groups) = pages.groups.clone() else {
        return html(
            &pages,
            StatusCode::NOT_FOUND,
            "Rooms are off",
            None,
            &format!(
                "<div class=\"box\"><div class=\"empty\"><h2>This relay has no rooms</h2>\
                 <p>Rooms are NIP-29 groups, and this relay does not keep groups.</p>\
                 <p><a href=\"{PREFIX}\">Back to the feed</a></p></div></div>"
            ),
        );
    };
    let Some(room) = groups.room(&id).await else {
        // No room, or a private one: the relay serves neither's metadata
        // to a visitor, so the page cannot tell them apart.
        return html(
            &pages,
            StatusCode::NOT_FOUND,
            "Room not found",
            None,
            &format!(
                "<div class=\"box\" data-sign-in><div class=\"empty\"><h2>No public room here</h2>\
                 <p>This relay holds no room <code>{}</code> that a visitor may see. A private room \
                 needs sign-in in a client: a member reads it in a Nostr client that signs in to this \
                 relay (NIP-42).</p><p><a href=\"{PREFIX}\">Back to the feed</a></p></div></div>",
                escape(&id)
            ),
        );
    };
    if !room.listed() {
        return html(
            &pages,
            StatusCode::OK,
            "Room needs sign-in",
            None,
            &sign_in_box(None, groups.signer),
        );
    }
    let mut body = String::from("<div class=\"room-head\">");
    if let Some(picture) = room
        .picture
        .as_deref()
        .and_then(|url| Media::new(url, Some("image/*")))
    {
        body.push_str(&picture.html("room-picture"));
    }
    body.push_str(&format!(
        "<div class=\"grow\"><h1 class=\"page-title\">#{}</h1>",
        escape(&room.name)
    ));
    if let Some(about) = &room.about {
        body.push_str(&format!("<p class=\"muted\">{}</p>", escape(about)));
    }
    body.push_str(&format!(
        "<div class=\"chips\"><span class=\"chip\">{}</span><span class=\"chip\">{}</span>\
         <span class=\"chip\">NIP-29 group on this relay</span></div></div>\
         <a class=\"btn\" href=\"nostr:{}\">Open in your client</a></div>",
        if room.private { "private" } else { "public" },
        if room.closed { "closed" } else { "open" },
        escape(&room.naddr(groups.signer)),
    ));
    if !room.readable() {
        body.push_str(&sign_in_box(Some(&room), groups.signer));
        return html(&pages, StatusCode::OK, &room.name, None, &body);
    }
    let store = &pages.store;
    let messages = in_room(store, &room.id, CHAT, TRANSCRIPT).await;
    let mut threads = in_room(store, &room.id, THREAD, THREADS).await;
    threads.reverse();
    let streams = room_streams(store, &room.id).await;
    let members = groups
        .raw
        .query(
            Filter::new()
                .kinds([Kind::from(GROUP_MEMBERS), Kind::from(GROUP_ADMINS)])
                .author(groups.signer)
                .identifier(room.id.clone()),
        )
        .await;
    let newest_of = |kind: u16| {
        newest(
            members
                .iter()
                .filter(|event| event.kind == Kind::from(kind) && d_tag(event) == room.id),
        )
    };
    let member_keys = newest_of(GROUP_MEMBERS)
        .map(listed_keys)
        .unwrap_or_default();
    let admin_keys = newest_of(GROUP_ADMINS).map(listed_keys).unwrap_or_default();
    let mut keys: Vec<PublicKey> = Vec::new();
    for key in messages
        .iter()
        .chain(threads.iter())
        .map(|event| event.pubkey)
        .chain(
            messages
                .iter()
                .flat_map(|event| content::mentioned_keys(&event.content)),
        )
        .chain(admin_keys.iter().copied())
    {
        if !keys.contains(&key) {
            keys.push(key);
        }
    }
    let profiles = Profiles::load(store, &keys).await;
    let names = profiles.names();

    if let Some(banner) = live::banner(&streams, chrome::now()) {
        body.push_str(&banner);
    }
    body.push_str("<div class=\"split\"><div class=\"main\">");
    body.push_str(&format!(
        "<section class=\"box\"><div class=\"box-header\"><span class=\"strong\">Chat</span>\
         <span class=\"muted small\">{} {}, oldest first (kind 9, NIP-C7)</span></div>\
         <div class=\"box-body\">{}</div><div class=\"box-footer\">{READ_ONLY_NOTE}</div></section>",
        messages.len(),
        plural(messages.len(), "message", "messages"),
        transcript_html(names, &messages),
    ));
    body.push_str("</div><aside class=\"sidebar\">");
    body.push_str("<div><h2>Threads</h2>");
    if threads.is_empty() {
        body.push_str("<p class=\"muted small\">No threads yet.</p>");
    } else {
        body.push_str("<ul>");
        for thread in &threads {
            let title = first_tag(thread, "title")
                .map(str::to_owned)
                .unwrap_or_else(|| content::first_line(&thread.content, 80));
            body.push_str(&format!(
                "<li data-id=\"{}\"><a href=\"{}\">{}</a><div class=\"muted small\">{} &middot; {}</div></li>",
                thread.id.to_hex(),
                escape(&crate::post::event_path(thread)),
                escape(if title.is_empty() { "Thread" } else { &title }),
                name(names, &thread.pubkey),
                ago(thread.created_at),
            ));
        }
        body.push_str("</ul>");
    }
    body.push_str("</div><div><h3>Admins</h3><ul>");
    for key in &admin_keys {
        body.push_str(&format!(
            "<li><a href=\"{}\">{} {}</a></li>",
            chrome::profile_path(key),
            avatar(&plain_name(names, key), true),
            name(names, key)
        ));
    }
    body.push_str(&format!(
        "</ul><p class=\"muted small\">{} {} in all (kind 39002, signed by this relay).</p></div>",
        member_keys.len(),
        plural(member_keys.len(), "member", "members"),
    ));
    body.push_str(
        "<div class=\"side-card\"><h2>Join or post</h2><p class=\"muted small\">This page writes nothing. \
         A client joins with a kind 9021 and posts a kind 9 with the room's <code>h</code> tag; each \
         is one paid write.</p></div></aside></div>",
    );
    html(&pages, StatusCode::OK, &room.name, None, &body)
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;

    use super::*;

    fn signed(keys: &Keys, kind: u16, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn a_rooms_metadata_counts_only_from_the_relays_key() {
        let (relay, client) = (Keys::generate(), Keys::generate());
        let tags: &[&[&str]] = &[&["d", "club"], &["name", "Club"], &["public"], &["open"]];
        let room = Room::read(&signed(&relay, GROUP_METADATA, tags), &relay.public_key()).unwrap();
        assert!(room.readable());
        assert_eq!(room.name, "Club");
        assert!(Room::read(&signed(&client, GROUP_METADATA, tags), &relay.public_key()).is_none());
    }
}
