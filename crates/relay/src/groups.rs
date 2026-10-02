//! NIP-29: relay-based groups (#219).
//!
//! It is off unless the operator turns it on, so a relay that was never told
//! about it keeps treating an `h` tag as any other tag. When on, a group is
//! made by a kind 9007 and kept as what the moderation events written to it
//! add up to; nothing is stored beyond those events and the metadata events
//! the relay signs from them, so the state is rebuilt from the file when the
//! relay starts.
//!
//! What the relay enforces, on the write port, before an event is stored:
//!
//! - an event with an `h` tag is written by a member of that group, judged by
//!   the event's author, the key whose signature the relay has verified. The
//!   connector proved the payment; it says nothing of who may write where;
//! - a moderation event (9000 put-user, 9001 remove-user, 9002 edit-metadata,
//!   9005 delete-event, 9008 delete-group) is written by someone with the
//!   role it needs: `admin` may do all of it, `moderator` may add plain
//!   members, remove plain members and delete events;
//! - 9007 creates a group that does not exist and makes its author an admin;
//!   9021 joins an open group at once and does nothing to a closed one (the
//!   request is kept for the admins to read); 9022 leaves;
//! - kinds 39000 to 39003 are the relay's own and are refused from clients.
//!
//! And what it publishes: after every change the group's four metadata
//! events (39000 metadata, 39001 admins, 39002 members, 39003 roles), signed
//! by the relay's key and addressed by the group id in their `d` tag.
//!
//! Reads: a group is `private` or `closed` as its metadata says, and either
//! makes its events readable only by a member who has authenticated (NIP-42,
//! which turning NIP-29 on turns on). A `private` group also hides its
//! metadata events. A `REQ` that names such a group in `#h` is closed
//! `auth-required:` until the connection has authenticated and `restricted:`
//! if the key is no member; whatever else a filter would return is held back
//! one event at a time.
//!
//! Not built: invites (9009, 9010), the `previous` timeline tags, and
//! custom roles with permissions of their own (a role name other than the two
//! above is kept and listed, and grants nothing).

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::sync::{Arc, PoisonError, RwLock};

use axum::http::StatusCode;
use nostr::event::{Event, EventBuilder, FinalizeEvent, Kind, Tag};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::{Keys, PublicKey};
use nostr::types::Timestamp;
use tokio::sync::{Mutex, MutexGuard};

use crate::auth::CLOSED_AUTH_REQUIRED;
use crate::clock::unix_seconds;
use crate::read_side::ReadSide;
use crate::store::Query;
use crate::{Store, VerifiedEvent};

const PUT_USER: u16 = 9000;
const REMOVE_USER: u16 = 9001;
const EDIT_METADATA: u16 = 9002;
const DELETE_EVENT: u16 = 9005;
const CREATE_GROUP: u16 = 9007;
const DELETE_GROUP: u16 = 9008;
const JOIN_REQUEST: u16 = 9021;
const LEAVE_REQUEST: u16 = 9022;
/// The moderation kinds, and the join and leave requests after them.
const MODERATION_KINDS: std::ops::RangeInclusive<u16> = 9000..=9022;

const METADATA: u16 = 39000;
const ADMINS: u16 = 39001;
const MEMBERS: u16 = 39002;
const ROLES: u16 = 39003;
const METADATA_KINDS: std::ops::RangeInclusive<u16> = METADATA..=ROLES;

const ADMIN: &str = "admin";
const MODERATOR: &str = "moderator";

/// The longest group id, in bytes.
const MAX_ID: usize = 64;

/// What a `CLOSED` says to an authenticated key that is no member.
const CLOSED_NOT_A_MEMBER: &str = "restricted: this group can be read by its members only";

/// The relay's key, which signs the metadata events.
#[derive(Clone)]
pub(crate) struct Signer(Keys);

impl Signer {
    pub(crate) fn new(keys: Keys) -> Self {
        Self(keys)
    }
}

impl std::fmt::Debug for Signer {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Signer(..)")
    }
}

/// A write the relay turns away: the status the connector is answered with
/// and the words.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Denied {
    pub(crate) status: StatusCode,
    pub(crate) reason: String,
}

fn denied(status: StatusCode, reason: impl Into<String>) -> Denied {
    Denied {
        status,
        reason: reason.into(),
    }
}

/// The first value of the first tag named `name`.
fn tag_value<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event
        .tags
        .iter()
        .find(|tag| tag.kind() == name)
        .and_then(Tag::content)
}

/// The group an event belongs to: the `h` tag of a group event, the `d` tag
/// of a metadata event.
fn group_of(event: &Event) -> Option<&str> {
    if METADATA_KINDS.contains(&event.kind.as_u16()) {
        tag_value(event, "d")
    } else {
        tag_value(event, "h")
    }
}

/// The keys the `p` tags of `event` name, with the roles each is given.
fn people(event: &Event) -> Vec<(PublicKey, BTreeSet<String>)> {
    event
        .tags
        .iter()
        .filter(|tag| tag.kind() == "p")
        .filter_map(|tag| {
            let slice = tag.as_slice();
            let key = PublicKey::from_hex(slice.get(1)?).ok()?;
            let roles = slice[2..]
                .iter()
                .filter(|role| !role.is_empty())
                .cloned()
                .collect();
            Some((key, roles))
        })
        .collect()
}

fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= MAX_ID
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}

#[derive(Debug, Clone)]
struct Group {
    name: String,
    about: Option<String>,
    picture: Option<String>,
    private: bool,
    closed: bool,
    /// Every member, and the roles each holds.
    members: BTreeMap<PublicKey, BTreeSet<String>>,
    /// The `created_at` of the metadata events last published, so the next
    /// replace them even within the same second.
    published_at: u64,
}

impl Group {
    fn new(id: &str) -> Self {
        Self {
            name: id.to_string(),
            about: None,
            picture: None,
            private: false,
            closed: false,
            members: BTreeMap::new(),
            published_at: 0,
        }
    }

    fn has_role(&self, key: &PublicKey, role: &str) -> bool {
        self.members
            .get(key)
            .is_some_and(|roles| roles.contains(role))
    }

    fn is_admin(&self, key: &PublicKey) -> bool {
        self.has_role(key, ADMIN)
    }

    fn is_moderator(&self, key: &PublicKey) -> bool {
        self.is_admin(key) || self.has_role(key, MODERATOR)
    }

    /// Whether the key holds a role that makes it more than a plain member.
    fn is_privileged(&self, key: &PublicKey) -> bool {
        self.has_role(key, ADMIN) || self.has_role(key, MODERATOR)
    }

    /// Read the metadata tags of an event (9007 and 9002 carry the same).
    fn edit(&mut self, event: &Event) {
        for tag in event.tags.iter() {
            let value = tag.content().map(str::to_string);
            match (tag.kind(), value) {
                ("name", Some(name)) => self.name = name,
                ("about", about) => self.about = about,
                ("picture", picture) => self.picture = picture,
                ("public", _) => self.private = false,
                ("private", _) => self.private = true,
                ("open", _) => self.closed = false,
                ("closed", _) => self.closed = true,
                _ => {}
            }
        }
    }

    /// Whether the content of the group is for members only.
    fn content_restricted(&self) -> bool {
        self.private || self.closed
    }

    fn includes_any(&self, keys: &HashSet<PublicKey>) -> bool {
        keys.iter().any(|key| self.members.contains_key(key))
    }
}

/// What a change to a group leaves for the async side to do.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Effect {
    Nothing,
    /// The group's metadata events are to be published again.
    Republish(String),
    /// Events of the group are to be deleted, those among `ids`.
    Retract {
        group: String,
        ids: Vec<String>,
    },
    /// The group is gone: its events are to be deleted.
    Dissolve(String),
}

/// Every group the relay holds.
#[derive(Debug, Default)]
pub(crate) struct Groups(BTreeMap<String, Group>);

impl Groups {
    /// Whether the group-ness of `event` is the relay's business.
    fn concerns(event: &Event) -> bool {
        let kind = event.kind.as_u16();
        MODERATION_KINDS.contains(&kind)
            || METADATA_KINDS.contains(&kind)
            || tag_value(event, "h").is_some()
    }

    /// Whether `event` may be written, given the groups as they stand.
    fn permit(&self, event: &Event) -> Result<(), Denied> {
        let kind = event.kind.as_u16();
        if METADATA_KINDS.contains(&kind) {
            return Err(denied(
                StatusCode::FORBIDDEN,
                format!("restricted: kind {kind} is written by the relay"),
            ));
        }
        let Some(id) = tag_value(event, "h") else {
            return Err(denied(
                StatusCode::UNPROCESSABLE_ENTITY,
                "a group event names its group in an h tag",
            ));
        };
        let author = &event.pubkey;
        if kind == CREATE_GROUP {
            if !valid_id(id) {
                return Err(denied(
                    StatusCode::UNPROCESSABLE_ENTITY,
                    format!("a group id is 1 to {MAX_ID} characters of a-z, 0-9, - and _"),
                ));
            }
            return match self.0.contains_key(id) {
                true => Err(denied(StatusCode::CONFLICT, "this group already exists")),
                false => Ok(()),
            };
        }
        let Some(group) = self.0.get(id) else {
            return Err(denied(StatusCode::NOT_FOUND, "no such group"));
        };
        let forbidden = |reason: &str| {
            Err(denied(
                StatusCode::FORBIDDEN,
                format!("restricted: {reason}"),
            ))
        };
        match kind {
            JOIN_REQUEST => Ok(()),
            LEAVE_REQUEST if group.members.contains_key(author) => Ok(()),
            LEAVE_REQUEST => forbidden("you are not a member of this group"),
            PUT_USER => {
                let people = people(event);
                if people.is_empty() {
                    return Err(denied(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "put-user names a p tag",
                    ));
                }
                let grants_roles = people.iter().any(|(_, roles)| !roles.is_empty());
                let alters_privileged = people.iter().any(|(key, _)| group.is_privileged(key));
                if group.is_admin(author)
                    || (group.is_moderator(author) && !grants_roles && !alters_privileged)
                {
                    Ok(())
                } else {
                    forbidden("adding members or granting roles needs a higher role")
                }
            }
            REMOVE_USER => {
                let people = people(event);
                if people.is_empty() {
                    return Err(denied(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "remove-user names a p tag",
                    ));
                }
                let removes_privileged = people.iter().any(|(key, _)| group.is_privileged(key));
                if group.is_admin(author) || (group.is_moderator(author) && !removes_privileged) {
                    Ok(())
                } else {
                    forbidden("removing this member needs a higher role")
                }
            }
            DELETE_EVENT => {
                if !event.tags.iter().any(|tag| tag.kind() == "e") {
                    return Err(denied(
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "delete-event names an e tag",
                    ));
                }
                if group.is_moderator(author) {
                    Ok(())
                } else {
                    forbidden("deleting an event needs a moderator")
                }
            }
            EDIT_METADATA | DELETE_GROUP => {
                if group.is_admin(author) {
                    Ok(())
                } else {
                    forbidden("this needs an admin of the group")
                }
            }
            other if MODERATION_KINDS.contains(&other) => Err(denied(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("moderation kind {other} is not supported by this relay"),
            )),
            _ if group.members.contains_key(author) => Ok(()),
            _ => forbidden("only a member can write to this group"),
        }
    }

    /// Fold a stored, permitted `event` into the groups.
    fn apply(&mut self, event: &Event) -> Effect {
        let Some(id) = tag_value(event, "h").map(str::to_string) else {
            return Effect::Nothing;
        };
        let kind = event.kind.as_u16();
        if kind == CREATE_GROUP {
            let mut group = Group::new(&id);
            group.edit(event);
            group
                .members
                .insert(event.pubkey, BTreeSet::from([ADMIN.to_string()]));
            self.0.insert(id.clone(), group);
            return Effect::Republish(id);
        }
        let Some(group) = self.0.get_mut(&id) else {
            return Effect::Nothing;
        };
        match kind {
            EDIT_METADATA => group.edit(event),
            PUT_USER => {
                for (key, roles) in people(event) {
                    group.members.insert(key, roles);
                }
            }
            REMOVE_USER => {
                for (key, _) in people(event) {
                    group.members.remove(&key);
                }
            }
            JOIN_REQUEST if !group.closed => {
                group.members.entry(event.pubkey).or_default();
            }
            LEAVE_REQUEST => {
                group.members.remove(&event.pubkey);
            }
            DELETE_EVENT => {
                let ids = event
                    .tags
                    .iter()
                    .filter(|tag| tag.kind() == "e")
                    .filter_map(|tag| tag.content().map(str::to_string))
                    .collect();
                return Effect::Retract { group: id, ids };
            }
            DELETE_GROUP => {
                self.0.remove(&id);
                return Effect::Dissolve(id);
            }
            _ => return Effect::Nothing,
        }
        Effect::Republish(id)
    }

    /// Whether `event` may be shown to a connection that has proven `keys`.
    fn may_read(&self, event: &Event, keys: &HashSet<PublicKey>) -> bool {
        let Some(group) = group_of(event).and_then(|id| self.0.get(id)) else {
            return true;
        };
        let metadata = METADATA_KINDS.contains(&event.kind.as_u16());
        let restricted = if metadata {
            group.private
        } else {
            group.content_restricted()
        };
        !restricted || group.includes_any(keys)
    }

    /// The `CLOSED` reason for a filter that names a group the connection
    /// may not read, if it does.
    fn refuse(&self, filter: &Filter, keys: &HashSet<PublicKey>) -> Option<&'static str> {
        let named = filter
            .generic_tags
            .iter()
            .filter(|(name, _)| **name == SingleLetterTag::LOWERCASE_H)
            .flat_map(|(_, ids)| ids.iter());
        for id in named {
            let Some(group) = self.0.get(id).filter(|group| group.content_restricted()) else {
                continue;
            };
            if keys.is_empty() {
                return Some(CLOSED_AUTH_REQUIRED);
            }
            if !group.includes_any(keys) {
                return Some(CLOSED_NOT_A_MEMBER);
            }
        }
        None
    }
}

/// What a connection asks the groups: who may read what.
#[derive(Debug, Clone)]
pub(crate) struct GroupView(Arc<RwLock<Groups>>);

impl GroupView {
    fn read(&self) -> std::sync::RwLockReadGuard<'_, Groups> {
        self.0.read().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn may_read(&self, event: &Event, keys: &HashSet<PublicKey>) -> bool {
        self.read().may_read(event, keys)
    }

    pub(crate) fn refuse(
        &self,
        filter: &Filter,
        keys: &HashSet<PublicKey>,
    ) -> Option<&'static str> {
        self.read().refuse(filter, keys)
    }
}

/// The write side of NIP-29: the groups, the order changes to them are made
/// in, and the key that signs their metadata.
#[derive(Debug, Clone)]
pub(crate) struct GroupRules {
    groups: Arc<RwLock<Groups>>,
    /// Held from the check of a group event to the change it makes, so two
    /// writes cannot both be judged against the groups as they were.
    turn: Arc<Mutex<()>>,
    signer: Signer,
}

impl GroupRules {
    /// The rules over the groups `stored` builds: the moderation events the
    /// store holds, in the order they were made.
    pub(crate) fn rebuilt(signer: Signer, store: &Store) -> Result<Self, crate::RelayError> {
        let mut stored = store.query_now(Filter::new().kinds(MODERATION_KINDS.map(Kind::from)))?;
        stored.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
        let mut groups = Groups::default();
        for event in &stored {
            if groups.permit(event).is_ok() {
                groups.apply(event);
            }
        }
        Ok(Self {
            groups: Arc::new(RwLock::new(groups)),
            turn: Arc::new(Mutex::new(())),
            signer,
        })
    }

    pub(crate) fn view(&self) -> GroupView {
        GroupView(Arc::clone(&self.groups))
    }

    /// Whether `event` is a group event, and so goes through [`Self::turn`]
    /// and [`Self::permit`].
    pub(crate) fn concerns(&self, event: &Event) -> bool {
        Groups::concerns(event)
    }

    /// Wait for a turn at the groups.
    pub(crate) async fn turn(&self) -> MutexGuard<'_, ()> {
        self.turn.lock().await
    }

    pub(crate) fn permit(&self, event: &Event) -> Result<(), Denied> {
        self.groups
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .permit(event)
    }

    /// `event` has been stored: make what it changes, and publish the
    /// metadata events that follow. A failure here is logged and not
    /// returned: the writer paid, and the event is stored.
    pub(crate) async fn accepted(&self, store: &Store, read_side: &ReadSide, event: &Event) {
        let effect = self
            .groups
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .apply(event);
        let outcome = match effect {
            Effect::Nothing => Ok(()),
            Effect::Republish(id) => self.republish(store, read_side, &id).await,
            Effect::Retract { group, ids } => retract(store, &group, ids).await,
            Effect::Dissolve(id) => dissolve(store, &id).await,
        };
        if let Err(error) = outcome {
            eprintln!(
                "groups: the change {} made was not completed: {error}",
                event.id
            );
        }
    }

    async fn republish(
        &self,
        store: &Store,
        read_side: &ReadSide,
        id: &str,
    ) -> Result<(), crate::RelayError> {
        let events = {
            let mut groups = self.groups.write().unwrap_or_else(PoisonError::into_inner);
            let Some(group) = groups.0.get_mut(id) else {
                return Ok(());
            };
            group.published_at = unix_seconds().max(group.published_at + 1);
            metadata_events(&self.signer, id, group)
        };
        for event in events {
            let event = VerifiedEvent::verify(event)?;
            let read_side = read_side.clone();
            store
                .save_then(&event, move |event| read_side.deliver(event))
                .await?;
        }
        Ok(())
    }
}

/// The group's four metadata events, signed by the relay.
fn metadata_events(signer: &Signer, id: &str, group: &Group) -> Vec<Event> {
    let tag = |parts: &[&str]| Tag::parse(parts.iter().copied()).ok();
    let mut details = vec![tag(&["d", id]), tag(&["name", &group.name])];
    details.push(group.picture.as_deref().and_then(|p| tag(&["picture", p])));
    details.push(group.about.as_deref().and_then(|a| tag(&["about", a])));
    details.push(tag(&[if group.private { "private" } else { "public" }]));
    details.push(tag(&[if group.closed { "closed" } else { "open" }]));

    let admins = group
        .members
        .iter()
        .filter(|(_, roles)| roles.contains(ADMIN))
        .map(|(key, roles)| {
            let mut parts = vec!["p".to_string(), key.to_hex()];
            parts.extend(roles.iter().cloned());
            Tag::parse(parts).ok()
        });
    let members = group
        .members
        .keys()
        .map(|key| Tag::parse(["p".to_string(), key.to_hex()]).ok());
    let roles = [
        tag(&["role", ADMIN, "may do everything in the group"]),
        tag(&[
            "role",
            MODERATOR,
            "may add and remove plain members and delete events",
        ]),
    ];
    let with_id = |rest: Vec<Option<Tag>>| {
        std::iter::once(tag(&["d", id]))
            .chain(rest)
            .flatten()
            .collect::<Vec<_>>()
    };
    let at = Timestamp::from(group.published_at);
    [
        (METADATA, details.into_iter().flatten().collect::<Vec<_>>()),
        (ADMINS, with_id(admins.collect())),
        (MEMBERS, with_id(members.collect())),
        (ROLES, with_id(roles.into())),
    ]
    .into_iter()
    .filter_map(|(kind, tags)| {
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags)
            .custom_created_at(at)
            .finalize(&signer.0)
            .ok()
    })
    .collect()
}

/// Delete the events among `ids` that belong to `group`.
async fn retract(store: &Store, group: &str, ids: Vec<String>) -> Result<(), crate::RelayError> {
    let held = store.query(in_group(group)).await?;
    let doomed = held
        .iter()
        .map(|event| event.id.to_hex())
        .filter(|id| ids.contains(id))
        .collect();
    store.remove(doomed).await?;
    Ok(())
}

/// Delete everything a dissolved group left in the store.
async fn dissolve(store: &Store, group: &str) -> Result<(), crate::RelayError> {
    let mut doomed: Vec<String> = store
        .query(in_group(group))
        .await?
        .iter()
        .map(|event| event.id.to_hex())
        .collect();
    let metadata = Query {
        filter: Filter::new().kinds(METADATA_KINDS.map(Kind::from)),
        multi_letter_tags: vec![("d".to_string(), HashSet::from([group.to_string()]))],
    };
    doomed.extend(store.query(metadata).await?.iter().map(|e| e.id.to_hex()));
    store.remove(doomed).await?;
    Ok(())
}

/// Everything stored with `group` in its `h` tag.
fn in_group(group: &str) -> Query {
    Query {
        filter: Filter::new(),
        multi_letter_tags: vec![("h".to_string(), HashSet::from([group.to_string()]))],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn event(keys: &Keys, kind: u16, tags: &[&[&str]]) -> Event {
        let tags = tags
            .iter()
            .map(|tag| Tag::parse(tag.iter().copied()).expect("a non-empty tag parses"));
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags)
            .finalize(keys)
            .expect("a generated key signs an event")
    }

    /// Permit and apply `event`, as the write side does.
    fn write(groups: &mut Groups, event: &Event) -> Result<Effect, Denied> {
        groups.permit(event)?;
        Ok(groups.apply(event))
    }

    #[test]
    fn a_group_is_created_once_and_its_author_is_admin() {
        let (owner, other) = (Keys::generate(), Keys::generate());
        let mut groups = Groups::default();
        let created = event(&owner, CREATE_GROUP, &[&["h", "club"], &["closed"]]);
        assert_eq!(
            write(&mut groups, &created),
            Ok(Effect::Republish("club".into()))
        );
        assert!(groups.0["club"].is_admin(&owner.public_key()));
        assert!(groups.0["club"].closed);

        let again = event(&other, CREATE_GROUP, &[&["h", "club"]]);
        assert_eq!(
            write(&mut groups, &again).expect_err("it exists").status,
            StatusCode::CONFLICT
        );
        let bad = event(&other, CREATE_GROUP, &[&["h", "Not Valid"]]);
        assert_eq!(
            write(&mut groups, &bad).expect_err("bad id").status,
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[test]
    fn membership_is_the_authors() {
        let (owner, other) = (Keys::generate(), Keys::generate());
        let mut groups = Groups::default();
        write(&mut groups, &event(&owner, CREATE_GROUP, &[&["h", "g"]])).expect("created");
        let chat = |keys| event(keys, 9, &[&["h", "g"]]);
        assert!(groups.permit(&chat(&owner)).is_ok());
        assert_eq!(
            groups
                .permit(&chat(&other))
                .expect_err("not a member")
                .status,
            StatusCode::FORBIDDEN
        );
        // Naming a member in a p tag does not make the author one.
        let borrowed = event(
            &other,
            9,
            &[&["h", "g"], &["p", &owner.public_key().to_hex()]],
        );
        assert!(groups.permit(&borrowed).is_err());
    }

    #[test]
    fn a_moderator_manages_plain_members_and_not_roles() {
        let (owner, moderator, plain, newcomer) = (
            Keys::generate(),
            Keys::generate(),
            Keys::generate(),
            Keys::generate(),
        );
        let hex = |keys: &Keys| keys.public_key().to_hex();
        let mut groups = Groups::default();
        write(&mut groups, &event(&owner, CREATE_GROUP, &[&["h", "g"]])).expect("created");
        let put = event(
            &owner,
            PUT_USER,
            &[
                &["h", "g"],
                &["p", &hex(&moderator), MODERATOR],
                &["p", &hex(&plain)],
            ],
        );
        write(&mut groups, &put).expect("the admin adds both");

        let add = event(
            &moderator,
            PUT_USER,
            &[&["h", "g"], &["p", &hex(&newcomer)]],
        );
        assert!(write(&mut groups, &add).is_ok());
        let promote = event(
            &moderator,
            PUT_USER,
            &[&["h", "g"], &["p", &hex(&plain), ADMIN]],
        );
        assert!(groups.permit(&promote).is_err());
        let evict_admin = event(
            &moderator,
            REMOVE_USER,
            &[&["h", "g"], &["p", &hex(&owner)]],
        );
        assert!(groups.permit(&evict_admin).is_err());
        let rename = event(&moderator, EDIT_METADATA, &[&["h", "g"], &["name", "x"]]);
        assert!(groups.permit(&rename).is_err());
        let evict = event(
            &moderator,
            REMOVE_USER,
            &[&["h", "g"], &["p", &hex(&plain)]],
        );
        assert!(write(&mut groups, &evict).is_ok());
        assert!(!groups.0["g"].members.contains_key(&plain.public_key()));
    }

    #[test]
    fn joining_is_free_in_an_open_group_and_does_nothing_in_a_closed_one() {
        let (owner, joiner) = (Keys::generate(), Keys::generate());
        let mut groups = Groups::default();
        write(&mut groups, &event(&owner, CREATE_GROUP, &[&["h", "open"]])).expect("created");
        write(
            &mut groups,
            &event(&owner, CREATE_GROUP, &[&["h", "shut"], &["closed"]]),
        )
        .expect("created");
        write(
            &mut groups,
            &event(&joiner, JOIN_REQUEST, &[&["h", "open"]]),
        )
        .expect("joins");
        let kept = write(
            &mut groups,
            &event(&joiner, JOIN_REQUEST, &[&["h", "shut"]]),
        )
        .expect("the request is kept");
        assert_eq!(kept, Effect::Nothing);
        assert!(groups.0["open"].members.contains_key(&joiner.public_key()));
        assert!(!groups.0["shut"].members.contains_key(&joiner.public_key()));
        write(
            &mut groups,
            &event(&joiner, LEAVE_REQUEST, &[&["h", "open"]]),
        )
        .expect("leaves");
        assert!(!groups.0["open"].members.contains_key(&joiner.public_key()));
    }

    #[test]
    fn the_relays_own_kinds_are_refused_and_an_unknown_group_is_not_found() {
        let keys = Keys::generate();
        let groups = Groups::default();
        let forged = event(&keys, METADATA, &[&["d", "g"]]);
        assert_eq!(
            groups.permit(&forged).expect_err("relay kind").status,
            StatusCode::FORBIDDEN
        );
        let chat = event(&keys, 9, &[&["h", "g"]]);
        assert_eq!(
            groups.permit(&chat).expect_err("no group").status,
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn closed_and_private_groups_are_read_by_members_who_proved_their_key() {
        let (owner, stranger) = (Keys::generate(), Keys::generate());
        let mut groups = Groups::default();
        write(
            &mut groups,
            &event(&owner, CREATE_GROUP, &[&["h", "shut"], &["closed"]]),
        )
        .expect("created");
        write(
            &mut groups,
            &event(&owner, CREATE_GROUP, &[&["h", "secret"], &["private"]]),
        )
        .expect("created");
        let proven = |keys: &Keys| HashSet::from([keys.public_key()]);
        let (none, member, outsider) = (HashSet::new(), proven(&owner), proven(&stranger));

        let chat = event(&owner, 9, &[&["h", "shut"]]);
        assert!(!groups.may_read(&chat, &none));
        assert!(!groups.may_read(&chat, &outsider));
        assert!(groups.may_read(&chat, &member));

        // Metadata of a closed group is discoverable, a private group's is not.
        let shut_meta = event(&owner, METADATA, &[&["d", "shut"]]);
        let secret_meta = event(&owner, METADATA, &[&["d", "secret"]]);
        assert!(groups.may_read(&shut_meta, &none));
        assert!(!groups.may_read(&secret_meta, &outsider));
        assert!(groups.may_read(&secret_meta, &member));

        let unrelated = event(&owner, 1, &[]);
        assert!(groups.may_read(&unrelated, &none));

        let asks = Filter::new().custom_tag(nostr::filter::SingleLetterTag::LOWERCASE_H, "shut");
        assert_eq!(groups.refuse(&asks, &none), Some(CLOSED_AUTH_REQUIRED));
        assert_eq!(groups.refuse(&asks, &outsider), Some(CLOSED_NOT_A_MEMBER));
        assert_eq!(groups.refuse(&asks, &member), None);
    }

    #[test]
    fn metadata_events_state_the_group() {
        let owner = Keys::generate();
        let mut groups = Groups::default();
        write(
            &mut groups,
            &event(&owner, CREATE_GROUP, &[&["h", "g"], &["name", "Agents"]]),
        )
        .expect("created");
        let mut group = groups.0["g"].clone();
        group.published_at = 1_700_000_000;
        let events = metadata_events(&Signer::new(Keys::generate()), "g", &group);
        let kinds: Vec<u16> = events.iter().map(|e| e.kind.as_u16()).collect();
        assert_eq!(kinds, [METADATA, ADMINS, MEMBERS, ROLES]);
        assert!(events.iter().all(|e| tag_value(e, "d") == Some("g")));
        assert_eq!(tag_value(&events[0], "name"), Some("Agents"));
        assert_eq!(
            tag_value(&events[1], "p"),
            Some(owner.public_key().to_hex().as_str())
        );
    }
}
