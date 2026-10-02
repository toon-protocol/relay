//! One read connection's NIP-01: what each message a client sends is answered
//! with, the subscriptions it holds and the live events they are sent.
//!
//! This is the relay's own subscription handling (#237). The relay was built
//! on `nostr-sdk`'s `local_relay`, which wraps every stream it is handed in a
//! WebSocket endpoint whose 128 KiB read buffer it gives no way to size, so
//! the relay kept its own endpoint in front of it and paid for both on every
//! connection. #185 names the way out taken here: keep only the `nostr`
//! protocol crate, which still reads a filter and matches an event against
//! one, and answer the messages in the relay.
//!
//! Nothing here touches a socket or the store. [`Session::client_sent`] says
//! what to send, or what to ask the store, and `read_side` does it; that is
//! what lets every rule below be tested without either.
//!
//! What a client is told (#185, compatibility contract):
//!
//! - a message that is not JSON, not an array, of a type the relay does not
//!   take, or a REQ without a string subscription id or with a filter that
//!   cannot be read, is a `NOTICE` that names which. `COUNT` and negentropy
//!   are types the relay does not take, as on the TypeScript relay;
//! - a REQ past the subscription limit or with more filters than the limit is
//!   a `NOTICE`, and opens nothing;
//! - a REQ the relay will not answer for another reason (an id too long, too
//!   many queries a minute, too many bytes of open requests) is a `CLOSED`
//!   that says why, and so is one the store could not answer;
//! - a REQ is answered from the store, every event once, newest first, then
//!   `EOSE`, and is a subscription from then on, whatever it asked for: one
//!   that names its events by id stays open when it has been sent them all;
//! - an `ids` or `authors` entry that is not a whole 64-character hex value
//!   matches nothing, instead of being a prefix;
//! - a tag filter on a key longer than one letter (`#ab`), which the protocol
//!   crate does not read, is a condition of the store's query, before the
//!   limit, and is applied to live events;
//! - an `EVENT` is answered `OK false` with the refusal that names the Write
//!   Edge, whatever the event: writes are paid and arrive on the write port;
//! - an `AUTH` is answered `OK false`: the relay issues no challenge (NIP-42
//!   is left off);
//! - a live event is one frame for each subscription it matches, serialised
//!   once for every connection ([`LiveFeed`]), and a connection that falls a
//!   whole feed behind has its subscriptions closed rather than carry on with
//!   a gap in them.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nostr::event::{Event, Kind};
use nostr::filter::{Filter, MatchEventOptions};
use serde_json::{Value, json};
use tokio::sync::broadcast;

use crate::Carriage;
use crate::connector::EdgeSlot;
use crate::document::write_refusal;
use crate::store::Query;

/// The most subscriptions one connection holds. Replacing one is not another.
pub(crate) const MAX_SUBSCRIPTIONS: usize = 20;
/// The most filters one REQ carries.
pub(crate) const MAX_FILTERS: usize = 10;
/// The most stored events any one filter is answered with, and the number a
/// filter that names no `limit` is answered with. One number: the stored
/// answers keep to it and the Relay Information Document states it as
/// `max_limit` and `default_limit`.
pub(crate) const MAX_LIMIT: usize = 500;

// The figures below are the ones the relay ran with on `local_relay`, kept so
// that taking over its work changes no limit a client has met.

/// The longest subscription id, in bytes: it is kept and echoed in every
/// frame of its subscription.
const MAX_SUBSCRIPTION_ID: usize = 250;
/// The most bytes of REQ messages one connection's open subscriptions were
/// asked in, together: what bounds the filters a connection makes the relay
/// keep and match every live event against.
const MAX_SUBSCRIPTION_BYTES: usize = 1024 * 1024;
/// How many frames of any kind a connection may send a minute.
pub(crate) const MESSAGES_PER_MINUTE: u32 = 6_000;
/// How many live events may wait for a connection that is not taking them,
/// before it has missed one.
const LIVE_BUFFER: usize = 1024;

/// What a subscription is closed with when its connection missed live events.
const CLOSED_OVERFLOW: &str =
    "error: live event buffer overflow; resubscribe to recover stored events";
/// What a subscription is closed with when the store could not answer it.
const CLOSED_STORE: &str = "error: the store could not be read";
/// What a REQ over a connection's allowance is closed with: it says what to do
/// instead, which is the paid way of not polling.
const CLOSED_CONNECTION_RATE: &str = "rate-limited: too many queries on this connection; slow down, or subscribe (a REQ stays open and streams live events) instead of polling";
/// What a REQ over its source address's allowance is closed with.
const CLOSED_SOURCE_RATE: &str = "rate-limited: too many queries from your address; slow down, or subscribe (a REQ stays open and streams live events) instead of polling";
/// What a binary frame is answered with.
pub(crate) const NOTICE_BINARY: &str = "binary messages are not processed by this relay";

/// What an `EVENT`'s refusal is written from: the Write Edge as it stands when
/// the `EVENT` arrives, and the carriage the operator states where the
/// connector states none.
#[derive(Debug, Clone, Default)]
pub(crate) struct Refusal {
    pub(crate) edge: EdgeSlot,
    pub(crate) write_carriage: Option<Carriage>,
}

impl Refusal {
    /// What a client that sends `EVENT` over WebSocket is told: the
    /// TypeScript relay's words, for a relay that does and does not know its
    /// Write Edge.
    fn words(&self) -> String {
        let refusal = write_refusal(self.edge.current().as_deref(), self.write_carriage);
        format!("restricted: {refusal}")
    }
}

/// An event on its way to every open subscription it matches.
#[derive(Debug)]
pub(crate) struct LiveEvent {
    event: Event,
    /// The event as a frame carries it, written once for every subscriber.
    json: String,
}

impl LiveEvent {
    fn new(event: &Event) -> Self {
        Self {
            event: event.clone(),
            json: event_json(event),
        }
    }
}

/// The feed of live events: published to once per event, listened to by every
/// connection. Cheap to clone; every clone is the same feed.
#[derive(Debug, Clone)]
pub(crate) struct LiveFeed(broadcast::Sender<Arc<LiveEvent>>);

impl LiveFeed {
    /// A feed nobody listens to yet.
    pub(crate) fn new() -> Self {
        Self(broadcast::channel(LIVE_BUFFER).0)
    }

    /// Hand `event` to every connection open now.
    pub(crate) fn publish(&self, event: &Event) {
        // An error only says nobody is connected.
        let _ = self.0.send(Arc::new(LiveEvent::new(event)));
    }

    /// Every event published from now on.
    pub(crate) fn listen(&self) -> broadcast::Receiver<Arc<LiveEvent>> {
        self.0.subscribe()
    }
}

fn event_json(event: &Event) -> String {
    serde_json::to_string(event)
        .expect("an event is strings, numbers and lists, which always serialize")
}

/// So many of something a minute: a whole minute's worth to begin with,
/// refilled evenly and never above a minute's worth.
#[derive(Debug)]
pub(crate) struct Allowance {
    per_minute: f64,
    left: f64,
    /// When one was last asked for.
    asked: Option<Instant>,
}

impl Allowance {
    pub(crate) fn new(per_minute: u32) -> Self {
        let per_minute = f64::from(per_minute);
        Self {
            per_minute,
            left: per_minute,
            asked: None,
        }
    }

    /// Take one at `now`. `false` when none is left.
    pub(crate) fn take(&mut self, now: Instant) -> bool {
        if let Some(asked) = self.asked {
            let minutes = now.saturating_duration_since(asked).as_secs_f64() / 60.0;
            self.left = (self.left + minutes * self.per_minute).min(self.per_minute);
        }
        self.asked = Some(now);
        if self.left < 1.0 {
            return false;
        }
        self.left -= 1.0;
        true
    }
}

/// One source address's allowance, shared by every connection it opens.
pub(crate) type SourceAllowance = Arc<Mutex<Allowance>>;

/// The allowances of the source addresses connected now, so that opening
/// another connection is not a way round a limit.
#[derive(Debug)]
pub(crate) struct Sources {
    per_minute: u32,
    held: Mutex<HashMap<IpAddr, SourceAllowance>>,
}

/// How many sources are held before the idle ones are let go.
const SWEEP_AT: usize = 1024;

impl Sources {
    pub(crate) fn new(per_minute: u32) -> Self {
        Self {
            per_minute,
            held: Mutex::default(),
        }
    }

    /// The allowance of the source `ip` is.
    pub(crate) fn of(&self, ip: IpAddr) -> SourceAllowance {
        // A poisoned lock only means another connection panicked mid-count;
        // the map is still a valid map.
        let mut held = self
            .held
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if held.len() >= SWEEP_AT && !held.contains_key(&ip) {
            // A source with no connection and no REQ for a minute has its
            // whole allowance back, which is what a new one starts with.
            let now = Instant::now();
            held.retain(|_, allowance| {
                Arc::strong_count(allowance) > 1
                    || allowance
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .asked
                        .is_some_and(|asked| {
                            now.saturating_duration_since(asked) < Duration::from_secs(60)
                        })
            });
        }
        Arc::clone(
            held.entry(ip)
                .or_insert_with(|| Arc::new(Mutex::new(Allowance::new(self.per_minute)))),
        )
    }
}

/// The values a filter's `#ab`-style keys accept, by key without the `#`.
type MultiLetterKeys = Vec<(String, HashSet<String>)>;

/// Remove from `filter` the tag keys the protocol crate does not read, and
/// return them. `None` when one of them is not a list, or `filter` is not an
/// object.
fn take_multi_letter_keys(filter: &mut Value) -> Option<MultiLetterKeys> {
    let object = filter.as_object_mut()?;
    let keys: Vec<String> = object
        .keys()
        .filter(|key| key.starts_with('#') && key.chars().count() > 2)
        .cloned()
        .collect();
    let mut multi = Vec::new();
    for key in keys {
        let values = object.remove(&key)?;
        let values = values
            .as_array()?
            .iter()
            .filter_map(|value| value.as_str().map(str::to_string))
            .collect();
        multi.push((key[1..].to_string(), values));
    }
    Some(multi)
}

/// Drop from `filter`'s `ids` and `authors` every entry that is not a whole
/// 64-character hex value. An entry left empty matches nothing, which is what
/// a prefix does on a relay that matches exactly.
fn keep_whole_values(filter: &mut Value) {
    for key in ["ids", "authors"] {
        if let Some(Value::Array(values)) = filter.get_mut(key) {
            values.retain(|value| {
                value.as_str().is_some_and(|hex| {
                    hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())
                })
            });
        }
    }
}

/// One filter of a subscription: what the protocol crate reads of it, and the
/// tag keys it does not.
#[derive(Debug)]
struct Wanted {
    base: Filter,
    /// `#ab`-style keys with the values they accept.
    multi: MultiLetterKeys,
}

impl Wanted {
    /// Read `filter` as a client wrote it. `None` when it is not a filter.
    fn read(mut filter: Value) -> Option<Self> {
        keep_whole_values(&mut filter);
        let multi = take_multi_letter_keys(&mut filter)?;
        let base = serde_json::from_value(filter).ok()?;
        Some(Self { base, multi })
    }

    /// Whether a live `event` is one this filter asks for: what the store
    /// would have answered it with, had the event been stored.
    fn matches(&self, event: &Event) -> bool {
        // A list that names nothing matches nothing, as in the store. The
        // protocol crate reads one as no condition at all, which would send
        // every live event to a filter whose only id was a prefix.
        let names_nothing = self.base.ids.as_ref().is_some_and(BTreeSet::is_empty)
            || self.base.authors.as_ref().is_some_and(BTreeSet::is_empty)
            || self.base.kinds.as_ref().is_some_and(BTreeSet::is_empty)
            || self
                .base
                .generic_tags
                .values()
                .any(|values| values.is_empty());
        !names_nothing
            && self.base.match_event(event, MatchEventOptions::new())
            && self.matches_multi_letter_keys(event)
    }

    fn matches_multi_letter_keys(&self, event: &Event) -> bool {
        self.multi.iter().all(|(name, values)| {
            event.tags.iter().any(|tag| {
                tag.as_slice().first() == Some(name)
                    && tag.content().is_some_and(|value| values.contains(value))
            })
        })
    }

    /// The filter as the store is asked: with the limit it is answered to,
    /// and the tag keys the protocol crate does not read.
    fn query(&self) -> Query {
        let mut filter = self.base.clone();
        let requested = filter
            .limit
            .unwrap_or_else(|| filter.ids.as_ref().map_or(MAX_LIMIT, |ids| ids.len()));
        filter.limit = Some(requested.min(MAX_LIMIT));
        Query {
            filter,
            multi_letter_tags: self.multi.clone(),
        }
    }
}

/// An open subscription.
#[derive(Debug)]
struct Subscription {
    filters: Vec<Wanted>,
    /// How each of its frames begins: `["EVENT","<id>",`.
    head: String,
    /// The bytes of the REQ that asked for it.
    size: usize,
}

impl Subscription {
    /// The frame that carries an event, given as JSON, to this subscription.
    fn frame(&self, event: &str) -> String {
        format!("{}{event}]", self.head)
    }
}

/// A REQ the relay will answer, once the store has been asked.
#[derive(Debug)]
pub(crate) struct Request {
    id: String,
    subscription: Subscription,
}

impl Request {
    /// The subscription id, for the log.
    pub(crate) fn id(&self) -> &str {
        &self.id
    }

    /// The questions to put to the store, one for each filter.
    pub(crate) fn queries(&self) -> Vec<Query> {
        self.subscription
            .filters
            .iter()
            .map(Wanted::query)
            .collect()
    }
}

/// What the relay does about one message from a client.
#[derive(Debug)]
pub(crate) enum Reply {
    /// Send these frames, in order. None, for a message that has no answer.
    Frames(Vec<String>),
    /// Ask the store what this request wants, then give what it found to
    /// [`Session::answered`], or tell [`Session::unanswered`] it could not.
    Ask(Request),
}

impl Reply {
    fn notice(text: impl AsRef<str>) -> Self {
        Self::Frames(vec![json!(["NOTICE", text.as_ref()]).to_string()])
    }
}

/// One connection's subscriptions and allowances.
#[derive(Debug)]
pub(crate) struct Session {
    refusal: Refusal,
    subscriptions: HashMap<String, Subscription>,
    queries: Allowance,
    /// The allowance of the address the connection came from.
    source: SourceAllowance,
    messages: Allowance,
}

impl Session {
    /// A connection that has said nothing yet, that is answered
    /// `queries_per_minute` REQs a minute and whatever its `source` has left.
    pub(crate) fn new(refusal: Refusal, queries_per_minute: u32, source: SourceAllowance) -> Self {
        Self {
            refusal,
            subscriptions: HashMap::new(),
            queries: Allowance::new(queries_per_minute),
            source,
            messages: Allowance::new(MESSAGES_PER_MINUTE),
        }
    }

    /// Count a frame the client sent at `now`, of any kind, so that no kind
    /// is a way round the limit. `false` when it is one too many: the
    /// connection is to be closed.
    pub(crate) fn frame_arrived(&mut self, now: Instant) -> bool {
        self.messages.take(now)
    }

    /// What to do about `text`, a message the client sent at `now`.
    pub(crate) fn client_sent(&mut self, text: &str, now: Instant) -> Reply {
        let items = match serde_json::from_str::<Value>(text) {
            Ok(Value::Array(items)) => items,
            Ok(_) => return Reply::notice("error: invalid message format, expected JSON array"),
            Err(_) => return Reply::notice("error: invalid JSON"),
        };
        match items.first().and_then(Value::as_str) {
            Some("REQ") => self.request(items, text.len(), now),
            Some("CLOSE") => {
                // No answer, and none for an id that is not open (NIP-01).
                if let Some(id) = items.get(1).and_then(Value::as_str) {
                    self.subscriptions.remove(id);
                }
                Reply::Frames(Vec::new())
            }
            Some("EVENT") => {
                let id = items
                    .get(1)
                    .and_then(|event| event.get("id"))
                    .cloned()
                    .unwrap_or(Value::Null);
                Reply::Frames(vec![
                    json!(["OK", id, false, self.refusal.words()]).to_string(),
                ])
            }
            Some("AUTH") => unsolicited_auth(items.into_iter().nth(1)),
            Some(other) => Reply::notice(format!("error: unknown message type: {other}")),
            None => Reply::notice("error: unknown message type: "),
        }
    }

    /// What to do about the REQ `items`, which arrived at `now` as `size`
    /// bytes.
    fn request(&mut self, items: Vec<Value>, size: usize, now: Instant) -> Reply {
        let Some(id) = items
            .get(1)
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_string)
        else {
            return Reply::notice("error: invalid subscription id");
        };
        let filters = items.into_iter().skip(2);
        if !self.subscriptions.contains_key(&id) && self.subscriptions.len() >= MAX_SUBSCRIPTIONS {
            return Reply::notice("error: too many subscriptions");
        }
        if filters.len() > MAX_FILTERS {
            return Reply::notice("error: too many filters");
        }
        if filters.len() == 0 {
            return Reply::notice("error: a REQ names at least one filter");
        }
        let Some(filters) = filters.map(Wanted::read).collect::<Option<Vec<_>>>() else {
            return Reply::notice("error: invalid filter");
        };

        if id.len() > MAX_SUBSCRIPTION_ID {
            return self.refuse(
                &id,
                &format!("blocked: subscription ID exceeds max length {MAX_SUBSCRIPTION_ID}"),
            );
        }
        if !self.queries.take(now) {
            return self.refuse(&id, CLOSED_CONNECTION_RATE);
        }
        let source_has_one = self
            .source
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take(now);
        if !source_has_one {
            return self.refuse(&id, CLOSED_SOURCE_RATE);
        }
        // The one it replaces gives its bytes back.
        let held: usize = self
            .subscriptions
            .iter()
            .filter(|(open, _)| **open != id)
            .map(|(_, subscription)| subscription.size)
            .sum();
        if held.saturating_add(size) > MAX_SUBSCRIPTION_BYTES {
            return self.refuse(
                &id,
                &format!(
                    "rate-limited: active subscriptions exceed max size {MAX_SUBSCRIPTION_BYTES} bytes"
                ),
            );
        }
        Reply::Ask(Request {
            subscription: Subscription {
                filters,
                head: format!("[\"EVENT\",{},", Value::from(id.as_str())),
                size,
            },
            id,
        })
    }

    /// End `id` with `reason`. A `CLOSED` says the subscription of that id is
    /// over (NIP-01), so one the refused REQ would have replaced ends too.
    fn refuse(&mut self, id: &str, reason: &str) -> Reply {
        Reply::Frames(vec![self.closed_frame(id, reason)])
    }

    /// Forget `id`, and the frame that tells the client it is over.
    fn closed_frame(&mut self, id: &str, reason: &str) -> String {
        self.subscriptions.remove(id);
        json!(["CLOSED", id, reason]).to_string()
    }

    /// The frames that answer `request` from what the store `found` for each
    /// of its queries: every event once, newest first and the lower id first
    /// among equals, then `EOSE`. The request is a subscription from here on.
    pub(crate) fn answered(
        &mut self,
        request: Request,
        found: Vec<Vec<Event>>,
        waiting: &[Arc<LiveEvent>],
    ) -> Vec<String> {
        let Request { id, subscription } = request;
        let mut seen = HashSet::new();
        let mut events: Vec<Event> = Vec::new();
        for found in found {
            for event in found {
                if seen.insert(event.id) {
                    events.push(event);
                }
            }
        }
        events.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
        let mut frames: Vec<String> = events
            .iter()
            .map(|event| subscription.frame(&event_json(event)))
            .chain(std::iter::once(json!(["EOSE", id]).to_string()))
            .collect();
        // What was published before the store was asked is delivered to the
        // open subscriptions as any live event is. The new one is sent it
        // only if its stored answer did not carry it, and one it replaces is
        // not sent it at all.
        self.subscriptions.remove(&id);
        for live in waiting {
            frames.extend(self.live_frames(live));
            if !seen.contains(&live.event.id)
                && subscription.filters.iter().any(|f| f.matches(&live.event))
            {
                frames.push(subscription.frame(&live.json));
            }
        }
        self.subscriptions.insert(id, subscription);
        frames
    }

    /// The connection fell behind the feed while `request` was being
    /// answered: it is closed with every open subscription.
    pub(crate) fn overflowed_during(&mut self, request: Request) -> Vec<String> {
        let Request { id, subscription } = request;
        self.subscriptions.insert(id, subscription);
        self.overflowed()
    }

    /// The frames that tell the client the store could not answer `request`.
    pub(crate) fn unanswered(&mut self, request: Request) -> Vec<String> {
        vec![self.closed_frame(&request.id, CLOSED_STORE)]
    }

    /// The frames `live` is to this connection: one for each open
    /// subscription it matches.
    pub(crate) fn live_frames<'a>(
        &'a self,
        live: &'a LiveEvent,
    ) -> impl Iterator<Item = String> + 'a {
        self.subscriptions
            .values()
            .filter(|subscription| subscription.filters.iter().any(|f| f.matches(&live.event)))
            .map(|subscription| subscription.frame(&live.json))
    }

    /// The connection missed live events, which cannot be matched any more:
    /// every subscription ends rather than carry on with a gap in it. The
    /// frames that tell the client so.
    pub(crate) fn overflowed(&mut self) -> Vec<String> {
        self.subscriptions
            .drain()
            .map(|(id, _)| json!(["CLOSED", id, CLOSED_OVERFLOW]).to_string())
            .collect()
    }
}

/// What an `AUTH` carrying `event` is answered with. The relay issues no
/// challenge, so there is none an `AUTH` could answer.
fn unsolicited_auth(event: Option<Value>) -> Reply {
    let Some(event) = event.and_then(|event| serde_json::from_value::<Event>(event).ok()) else {
        return Reply::notice("error: invalid AUTH event");
    };
    let reason = if event.kind != Kind::Authentication {
        "invalid authentication event kind"
    } else if event.tags.challenge().is_some() {
        "received invalid challenge"
    } else {
        "challenge not found"
    };
    Reply::Frames(vec![
        json!(["OK", event.id, false, format!("auth-required: {reason}")]).to_string(),
    ])
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    /// The per-connection allowance the tests run with.
    const QUERIES_PER_MINUTE: u32 = 1_200;

    fn session() -> Session {
        Session::new(
            Refusal::default(),
            QUERIES_PER_MINUTE,
            Arc::new(Mutex::new(Allowance::new(u32::MAX))),
        )
    }

    /// The frames `text` is answered with, when it asks the store nothing.
    fn said(session: &mut Session, text: &str) -> Vec<Value> {
        match session.client_sent(text, Instant::now()) {
            Reply::Frames(frames) => frames
                .iter()
                .map(|frame| serde_json::from_str(frame).expect("a relay frame is JSON"))
                .collect(),
            Reply::Ask(request) => panic!("{text} asks the store for {}", request.id()),
        }
    }

    /// The request `text` asks the store for.
    fn asked(session: &mut Session, text: &str) -> Request {
        match session.client_sent(text, Instant::now()) {
            Reply::Ask(request) => request,
            Reply::Frames(frames) => panic!("{text} is answered {frames:?}"),
        }
    }

    /// Open the subscription `text` asks for, on a store that holds nothing.
    fn open(session: &mut Session, text: &str) {
        let request = asked(session, text);
        let found = vec![Vec::new(); request.queries().len()];
        session.answered(request, found, &[]);
    }

    /// A session holding each of `requests` as an open subscription.
    fn subscribed(requests: &[&str]) -> Session {
        let mut session = session();
        for text in requests {
            open(&mut session, text);
        }
        session
    }

    /// A signed event of `kind` carrying `tags`.
    fn event(kind: u16, tags: &[&[&str]]) -> Event {
        event_at(kind, 1_700_000_000, tags)
    }

    fn event_at(kind: u16, created_at: u64, tags: &[&[&str]]) -> Event {
        use nostr::event::{EventBuilder, FinalizeEvent, Tag};
        use nostr::types::Timestamp;
        let tags = tags
            .iter()
            .map(|tag| Tag::parse(tag.iter().copied()).expect("a non-empty tag parses"));
        EventBuilder::new(Kind::from(kind), "an event")
            .tags(tags)
            .custom_created_at(Timestamp::from(created_at))
            .finalize(&nostr::key::Keys::generate())
            .expect("a generated key signs an event")
    }

    fn live(session: &Session, event: &Event) -> Vec<String> {
        let mut frames: Vec<String> = session.live_frames(&LiveEvent::new(event)).collect();
        frames.sort();
        frames
    }

    fn notice(text: &str) -> Vec<Value> {
        vec![json!(["NOTICE", text])]
    }

    #[test]
    fn a_request_is_put_to_the_store_one_question_for_each_filter() {
        let mut session = session();
        let request = asked(&mut session, r#"["REQ","a",{"kinds":[1]},{"kinds":[7]}]"#);
        assert_eq!(request.id(), "a");
        let kinds: Vec<_> = request
            .queries()
            .into_iter()
            .map(|query| query.filter.kinds.expect("kinds were named"))
            .collect();
        assert_eq!(
            kinds,
            vec![
                BTreeSet::from([Kind::from(1)]),
                BTreeSet::from([Kind::from(7)])
            ]
        );
    }

    #[test]
    fn a_stored_answer_is_every_event_once_newest_first_and_then_eose() {
        let mut session = session();
        let request = asked(&mut session, r#"["REQ","a",{"kinds":[1]},{}]"#);
        let (old, new) = (event_at(1, 100, &[]), event_at(1, 200, &[]));
        let mut twins = [event_at(1, 150, &[]), event_at(1, 150, &[])];
        twins.sort_by_key(|event| event.id);
        let found = vec![
            vec![old.clone(), new.clone()],
            vec![twins[1].clone(), old.clone(), twins[0].clone()],
        ];
        let frames: Vec<Value> = session
            .answered(request, found, &[])
            .iter()
            .map(|frame| serde_json::from_str(frame).expect("JSON"))
            .collect();
        assert_eq!(
            frames,
            vec![
                json!(["EVENT", "a", new]),
                json!(["EVENT", "a", twins[0]]),
                json!(["EVENT", "a", twins[1]]),
                json!(["EVENT", "a", old]),
                json!(["EOSE", "a"]),
            ]
        );
    }

    #[test]
    fn a_filter_without_a_limit_is_asked_with_the_default_and_none_above_the_cap() {
        let mut session = session();
        let whole = "ab".repeat(32);
        let request = asked(
            &mut session,
            &format!(
                r#"["REQ","a",{{"kinds":[1]}},{{"limit":3}},{{"limit":9999}},{{"ids":["{whole}"]}}]"#
            ),
        );
        let limits: Vec<_> = request.queries().iter().map(|q| q.filter.limit).collect();
        assert_eq!(
            limits,
            vec![Some(MAX_LIMIT), Some(3), Some(MAX_LIMIT), Some(1)]
        );
    }

    #[test]
    fn every_event_is_refused_whatever_it_is() {
        let mut session = session();
        let refusal = Refusal::default().words();
        for (text, id) in [
            (r#"["EVENT",{"id":"abc","sig":"bad"}]"#, json!("abc")),
            (
                r#"["EVENT",{"id":"def","tags":[["-"]],"kind":1}]"#,
                json!("def"),
            ),
            (r#"["EVENT","not an event"]"#, Value::Null),
        ] {
            assert_eq!(
                said(&mut session, text),
                vec![json!(["OK", id, false, refusal])],
                "{text}"
            );
        }
    }

    #[test]
    fn what_is_not_a_message_is_named_in_a_notice_of_its_own() {
        let mut session = session();
        for (text, words) in [
            ("{not json", "error: invalid JSON"),
            (
                r#"{"a":1}"#,
                "error: invalid message format, expected JSON array",
            ),
            (r#"["BOGUS","x"]"#, "error: unknown message type: BOGUS"),
            (r#"[7]"#, "error: unknown message type: "),
            (r#"["REQ",7,{}]"#, "error: invalid subscription id"),
            (r#"["REQ","",{}]"#, "error: invalid subscription id"),
        ] {
            assert_eq!(said(&mut session, text), notice(words), "{text}");
        }
    }

    #[test]
    fn a_request_that_names_no_filter_it_can_read_is_a_notice_and_no_subscription() {
        let mut session = session();
        for (text, words) in [
            (r#"["REQ","a"]"#, "error: a REQ names at least one filter"),
            (r#"["REQ","a",5]"#, "error: invalid filter"),
            (r#"["REQ","a",{"limit":-1}]"#, "error: invalid filter"),
            (r#"["REQ","a",{"kinds":["x"]}]"#, "error: invalid filter"),
            (r#"["REQ","a",{},{"kinds":"x"}]"#, "error: invalid filter"),
            (r##"["REQ","a",{"#ab":"x"}]"##, "error: invalid filter"),
        ] {
            assert_eq!(said(&mut session, text), notice(words), "{text}");
            assert!(live(&session, &event(1, &[])).is_empty(), "{text}");
        }
    }

    #[test]
    fn count_and_negentropy_are_not_message_types_this_relay_takes() {
        let mut session = session();
        for (text, kind) in [
            (r#"["COUNT","c",{}]"#, "COUNT"),
            (r#"["NEG-OPEN","n",{},"00"]"#, "NEG-OPEN"),
            (r#"["NEG-MSG","n","00"]"#, "NEG-MSG"),
            (r#"["NEG-CLOSE","n"]"#, "NEG-CLOSE"),
        ] {
            assert_eq!(
                said(&mut session, text),
                notice(&format!("error: unknown message type: {kind}")),
            );
        }
    }

    #[test]
    fn an_auth_is_refused_because_no_challenge_was_issued() {
        let mut session = session();
        let with_challenge = event(22242, &[&["challenge", "not one this relay issued"]]);
        let without = event(22242, &[]);
        let another_kind = event(1, &[&["challenge", "x"]]);
        for (event, reason) in [
            (with_challenge, "auth-required: received invalid challenge"),
            (without, "auth-required: challenge not found"),
            (
                another_kind,
                "auth-required: invalid authentication event kind",
            ),
        ] {
            assert_eq!(
                said(&mut session, &json!(["AUTH", event]).to_string()),
                vec![json!(["OK", event.id, false, reason])]
            );
        }
        assert_eq!(
            said(&mut session, r#"["AUTH","a challenge"]"#),
            notice("error: invalid AUTH event")
        );
    }

    #[test]
    fn the_subscription_after_the_limit_is_a_notice_but_replacing_one_is_not() {
        let mut session = session();
        for i in 0..MAX_SUBSCRIPTIONS {
            open(&mut session, &format!(r#"["REQ","s{i}",{{}}]"#));
        }
        assert_eq!(
            said(&mut session, r#"["REQ","more",{}]"#),
            notice("error: too many subscriptions")
        );
        open(&mut session, r#"["REQ","s0",{"kinds":[1]}]"#);
    }

    #[test]
    fn a_closed_subscription_frees_its_place_and_closing_has_no_answer() {
        let mut session = session();
        for i in 0..MAX_SUBSCRIPTIONS {
            open(&mut session, &format!(r#"["REQ","s{i}",{{}}]"#));
        }
        for text in [
            r#"["CLOSE","s0"]"#,
            r#"["CLOSE","never opened"]"#,
            r#"["CLOSE"]"#,
        ] {
            assert_eq!(said(&mut session, text), Vec::<Value>::new(), "{text}");
        }
        open(&mut session, r#"["REQ","a",{}]"#);
        assert_eq!(
            said(&mut session, r#"["REQ","b",{}]"#),
            notice("error: too many subscriptions")
        );
    }

    #[test]
    fn a_request_with_too_many_filters_is_a_notice_and_one_at_the_limit_is_not() {
        let mut session = session();
        let request = |filters: usize| format!(r#"["REQ","f",{}]"#, vec!["{}"; filters].join(","));
        assert_eq!(
            said(&mut session, &request(MAX_FILTERS + 1)),
            notice("error: too many filters")
        );
        assert!(live(&session, &event(1, &[])).is_empty(), "nothing opened");
        open(&mut session, &request(MAX_FILTERS));
    }

    #[test]
    fn a_request_by_ids_stays_open_when_it_has_been_sent_them_all() {
        let mut session = session();
        let stored = event(1, &[]);
        let request = asked(
            &mut session,
            &format!(r#"["REQ","a",{{"ids":["{}"]}}]"#, stored.id.to_hex()),
        );
        let frames = session.answered(request, vec![vec![stored.clone()]], &[]);
        assert_eq!(frames.len(), 2, "the event and EOSE, and no CLOSED");
        assert_eq!(live(&session, &stored).len(), 1, "and it is still open");
    }

    #[test]
    fn an_id_or_author_that_is_not_a_whole_value_matches_nothing_stored_or_live() {
        let mut session = session();
        let event = event(1, &[]);
        let (id, author) = (event.id.to_hex(), event.pubkey.to_hex());
        let request = asked(
            &mut session,
            &format!(
                r#"["REQ","p",{{"ids":["{}","{id}"]}},{{"authors":["{}"]}},{{"ids":["abcd"]}}]"#,
                &id[..16],
                &author[..16]
            ),
        );
        let queries = request.queries();
        assert_eq!(queries[0].filter.ids, Some(BTreeSet::from([event.id])));
        assert_eq!(queries[1].filter.authors, Some(BTreeSet::new()));
        assert_eq!(queries[2].filter.ids, Some(BTreeSet::new()));
        session.answered(request, vec![Vec::new(); 3], &[]);
        assert_eq!(live(&session, &event).len(), 1, "its whole id is named");

        let mut session = subscribed(&[r#"["REQ","p",{"ids":["abcd"]},{"authors":["abcd"]}]"#]);
        assert!(live(&session, &event).is_empty(), "a prefix names nothing");
        open(&mut session, r##"["REQ","k",{"kinds":[]},{"#t":[]}]"##);
        assert!(live(&session, &event).is_empty(), "nor does an empty list");
    }

    #[test]
    fn a_multi_letter_key_is_put_to_the_store_and_applied_to_live_events() {
        let mut session = session();
        let request = asked(&mut session, r##"["REQ","a",{"kinds":[1],"#ab":["x"]}]"##);
        assert_eq!(request.queries()[0].filter.generic_tags.len(), 0);
        assert_eq!(request.queries()[0].multi_letter_tags.len(), 1);
        let (hit, miss) = (event(1, &[&["ab", "x"]]), event(1, &[&["ab", "y"]]));
        let frames = session.answered(request, vec![vec![hit.clone()]], &[]);
        assert_eq!(frames.len(), 2, "what the store found, and EOSE");
        assert!(frames[0].contains(&hit.id.to_hex()));

        assert_eq!(live(&session, &hit).len(), 1);
        assert!(live(&session, &miss).is_empty());
        assert!(live(&session, &event(7, &[&["ab", "x"]])).is_empty());
    }

    #[test]
    fn a_multi_letter_key_of_one_filter_does_not_hide_what_another_filter_found() {
        let mut session = session();
        let request = asked(&mut session, r##"["REQ","a",{"#ab":["x"]},{"kinds":[1]}]"##);
        let plain = event(1, &[]);
        let frames = session.answered(request, vec![vec![plain.clone()], vec![plain]], &[]);
        assert_eq!(frames.len(), 2, "the second filter asked for it");
    }

    #[test]
    fn an_event_found_by_two_filters_and_waiting_in_the_feed_is_sent_once() {
        let mut session = session();
        let request = asked(&mut session, r#"["REQ","a",{"kinds":[1]},{}]"#);
        let saved = event(1, &[]);
        // Saved between the two queries: only the second found it, and the
        // feed already held it. And one the answer did not carry is sent live.
        let later = event(1, &[&["t", "later"]]);
        let waiting = [
            Arc::new(LiveEvent::new(&saved)),
            Arc::new(LiveEvent::new(&later)),
        ];
        let frames = session.answered(request, vec![Vec::new(), vec![saved.clone()]], &waiting);
        assert_eq!(
            frames,
            vec![
                format!(r#"["EVENT","a",{}]"#, event_json(&saved)),
                r#"["EOSE","a"]"#.to_string(),
                format!(r#"["EVENT","a",{}]"#, event_json(&later)),
            ]
        );
    }

    #[test]
    fn a_waiting_event_is_not_sent_to_the_subscription_a_request_replaces() {
        let mut session = subscribed(&[r#"["REQ","a",{}]"#]);
        let request = asked(&mut session, r#"["REQ","a",{"kinds":[1]}]"#);
        let saved = event(1, &[]);
        let waiting = [Arc::new(LiveEvent::new(&saved))];
        let frames = session.answered(request, vec![vec![saved.clone()]], &waiting);
        assert_eq!(
            frames,
            vec![
                format!(r#"["EVENT","a",{}]"#, event_json(&saved)),
                r#"["EOSE","a"]"#.to_string(),
            ]
        );
    }

    #[test]
    fn a_live_event_is_one_frame_for_each_subscription_it_matches() {
        let session = subscribed(&[
            r#"["REQ","a",{"kinds":[1]}]"#,
            r#"["REQ","b",{"kinds":[7]},{"kinds":[1]},{}]"#,
            r#"["REQ","c",{"kinds":[7]}]"#,
        ]);
        let event = event(1, &[]);
        let json = serde_json::to_string(&event).expect("JSON");
        assert_eq!(
            live(&session, &event),
            vec![
                format!(r#"["EVENT","a",{json}]"#),
                format!(r#"["EVENT","b",{json}]"#)
            ]
        );
    }

    #[test]
    fn a_frame_quotes_its_subscription_id_as_json_does() {
        let mut session = session();
        let request = asked(&mut session, r#"["REQ","a\"\\b",{}]"#);
        let event = event(1, &[]);
        let stored = session.answered(request, vec![vec![event.clone()]], &[]);
        for frame in [stored[0].clone(), live(&session, &event).remove(0)] {
            let frame: Value = serde_json::from_str(&frame).expect("a frame is JSON");
            assert_eq!(frame[1], json!("a\"\\b"));
        }
    }

    #[test]
    fn a_request_is_no_subscription_until_the_store_has_answered_it() {
        let mut session = session();
        let request = asked(&mut session, r#"["REQ","a",{}]"#);
        assert!(live(&session, &event(1, &[])).is_empty());
        session.answered(request, vec![Vec::new()], &[]);
        assert_eq!(live(&session, &event(1, &[])).len(), 1);
    }

    #[test]
    fn a_replaced_subscription_hears_only_what_the_new_request_asks_for() {
        let mut session = subscribed(&[r#"["REQ","a",{"kinds":[1]}]"#]);
        open(&mut session, r#"["REQ","a",{"kinds":[7]}]"#);
        assert!(live(&session, &event(1, &[])).is_empty());
        assert_eq!(live(&session, &event(7, &[])).len(), 1);
    }

    #[test]
    fn a_request_the_store_could_not_answer_is_closed_and_no_subscription() {
        let mut session = subscribed(&[r#"["REQ","a",{"kinds":[1]}]"#]);
        let request = asked(&mut session, r#"["REQ","a",{"kinds":[7]}]"#);
        assert_eq!(
            session.unanswered(request),
            vec![json!(["CLOSED", "a", CLOSED_STORE]).to_string()]
        );
        assert!(live(&session, &event(1, &[])).is_empty());
        assert!(live(&session, &event(7, &[])).is_empty());
    }

    #[test]
    fn a_subscription_id_longer_than_the_limit_is_closed() {
        let mut session = session();
        let at_limit = "x".repeat(MAX_SUBSCRIPTION_ID);
        open(&mut session, &json!(["REQ", at_limit, {}]).to_string());
        let over = "x".repeat(MAX_SUBSCRIPTION_ID + 1);
        assert_eq!(
            said(&mut session, &json!(["REQ", over, {}]).to_string()),
            vec![json!([
                "CLOSED",
                over,
                "blocked: subscription ID exceeds max length 250"
            ])]
        );
    }

    #[test]
    fn requests_past_a_minutes_worth_are_closed_until_the_allowance_refills() {
        let mut session = session();
        let start = Instant::now();
        let text = r#"["REQ","a",{}]"#;
        for _ in 0..QUERIES_PER_MINUTE {
            assert!(matches!(session.client_sent(text, start), Reply::Ask(_)));
        }
        let Reply::Frames(frames) = session.client_sent(text, start) else {
            panic!("one more than a minute's worth is refused");
        };
        assert_eq!(
            frames,
            vec![json!(["CLOSED", "a", CLOSED_CONNECTION_RATE]).to_string()]
        );
        // A twentieth of a second is one more at 1200 a minute.
        let later = start + Duration::from_millis(50);
        assert!(matches!(session.client_sent(text, later), Reply::Ask(_)));
        assert!(matches!(session.client_sent(text, later), Reply::Frames(_)));
    }

    #[test]
    fn connections_of_one_source_share_its_allowance_and_another_source_has_its_own() {
        let sources = Sources::new(3);
        let ip = |last: u8| IpAddr::from([10, 0, 0, last]);
        let connect =
            |last: u8| Session::new(Refusal::default(), QUERIES_PER_MINUTE, sources.of(ip(last)));
        let (mut first, mut second, mut stranger) = (connect(1), connect(1), connect(2));
        let now = Instant::now();
        let text = r#"["REQ","a",{}]"#;
        assert!(matches!(first.client_sent(text, now), Reply::Ask(_)));
        assert!(matches!(second.client_sent(text, now), Reply::Ask(_)));
        assert!(matches!(first.client_sent(text, now), Reply::Ask(_)));
        let Reply::Frames(frames) = second.client_sent(text, now) else {
            panic!("the source had three, and used them over two connections");
        };
        assert_eq!(
            frames,
            vec![json!(["CLOSED", "a", CLOSED_SOURCE_RATE]).to_string()]
        );
        assert!(matches!(stranger.client_sent(text, now), Reply::Ask(_)));
        // A new connection does not reset what the source has used.
        assert!(matches!(
            connect(1).client_sent(text, now),
            Reply::Frames(_)
        ));
    }

    #[test]
    fn a_refusal_says_to_slow_down_or_subscribe() {
        for words in [CLOSED_CONNECTION_RATE, CLOSED_SOURCE_RATE] {
            assert!(words.starts_with("rate-limited: "));
            assert!(words.contains("slow down") && words.contains("subscribe"));
        }
    }

    #[test]
    fn a_refused_request_ends_the_subscription_it_would_have_replaced() {
        let mut session = subscribed(&[r#"["REQ","a",{}]"#]);
        let start = Instant::now();
        while matches!(
            session.client_sent(r#"["REQ","b",{}]"#, start),
            Reply::Ask(_)
        ) {}
        assert_eq!(live(&session, &event(1, &[])).len(), 1, "a is still open");
        assert!(matches!(
            session.client_sent(r#"["REQ","a",{}]"#, start),
            Reply::Frames(_)
        ));
        assert!(
            live(&session, &event(1, &[])).is_empty(),
            "a was told it is closed"
        );
    }

    #[test]
    fn open_requests_past_the_byte_budget_are_closed_and_a_replacement_gives_its_bytes_back() {
        let mut session = session();
        let request = |id: &str, bytes: usize| {
            let text = json!(["REQ", id, { "#t": ["x".repeat(bytes)] }]).to_string();
            assert!(text.len() > bytes);
            text
        };
        let most = MAX_SUBSCRIPTION_BYTES - 1024;
        open(&mut session, &request("big", most));
        assert_eq!(
            said(&mut session, &request("more", 1024)),
            vec![json!([
                "CLOSED",
                "more",
                "rate-limited: active subscriptions exceed max size 1048576 bytes"
            ])]
        );
        open(&mut session, &request("big", most));
        said(&mut session, r#"["CLOSE","big"]"#);
        open(&mut session, &request("more", 1024));
    }

    #[test]
    fn frames_past_a_minutes_worth_are_one_too_many() {
        let mut session = session();
        let start = Instant::now();
        for _ in 0..MESSAGES_PER_MINUTE {
            assert!(session.frame_arrived(start));
        }
        assert!(!session.frame_arrived(start));
        assert!(session.frame_arrived(start + Duration::from_secs(1)));
    }

    #[test]
    fn an_allowance_never_holds_more_than_a_minutes_worth() {
        let mut allowance = Allowance::new(2);
        let start = Instant::now();
        assert!(allowance.take(start));
        let much_later = start + Duration::from_secs(3600);
        assert!(allowance.take(much_later));
        assert!(allowance.take(much_later));
        assert!(!allowance.take(much_later));
    }

    #[test]
    fn a_connection_that_missed_live_events_loses_every_subscription() {
        let mut session = subscribed(&[r#"["REQ","a",{}]"#, r#"["REQ","b",{}]"#]);
        let mut closed = session.overflowed();
        closed.sort();
        assert_eq!(
            closed,
            vec![
                json!(["CLOSED", "a", CLOSED_OVERFLOW]).to_string(),
                json!(["CLOSED", "b", CLOSED_OVERFLOW]).to_string()
            ]
        );
        assert!(live(&session, &event(1, &[])).is_empty());
        open(&mut session, r#"["REQ","a",{}]"#);
    }
}
