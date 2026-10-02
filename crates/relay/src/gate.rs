//! The gate between a client and the framework: what the relay says to a
//! connection before the framework hears it.
//!
//! The framework answers a REQ it dislikes with `CLOSED`, accepts an empty
//! subscription id and, past its connection cap, refuses without a close
//! code. The TypeScript relay's clients expect something else (#185,
//! compatibility contract), so the relay terminates each client's WebSocket
//! itself and speaks to the framework over an in-memory pipe, forwarding what
//! the framework should hear and answering the rest:
//!
//! - a message that is not JSON, not an array, of an unknown type or a REQ
//!   without a string subscription id is a `NOTICE` that names which;
//! - a `CLOSED` with no reason is not passed on: the framework sends one when
//!   a by-`ids` subscription has found all it asked for, and the subscription
//!   stays open, as on the TypeScript relay;
//! - an empty subscription id, a REQ past the subscription limit and a REQ
//!   with more filters than the limit are each a `NOTICE`, and the REQ goes
//!   no further;
//! - an `ids` or `authors` entry that is not a whole 64-character hex value
//!   matches nothing, instead of being a prefix (or a parse failure that the
//!   framework answers with a `NOTICE` and no `EOSE`);
//! - an `EVENT` is answered `OK false` with the refusal that names the Write
//!   Edge, whatever the event: the framework would answer one with a bad id,
//!   a bad signature or an expiry in its own words, and a protected (NIP-70)
//!   one with an `AUTH` challenge, before its write policy is asked;
//! - a tag filter on a key longer than one letter (`#ab`), which the framework
//!   ignores, is applied here, to the stored answer and to every live event;
//! - while NIP-40 expiration is not enforced, the stored answer is read from
//!   the store here, because the framework leaves an expired event out of
//!   every one whatever the store returns. Both cases hold the framework's
//!   own stored answer back and send the store's in its place, in NIP-01
//!   order, before the `EOSE` the framework sent;
//! - a connection past the cap is closed with 1013.
//!
//! Live events do not cross the framework at all: the gate delivers them
//! (#232). The framework hears every `REQ` and gives its stored answer; the
//! gate keeps the same filters and matches each new event against them
//! itself, from one feed every connection listens to ([`LiveFeed`]), with the
//! event serialised once for all of them. Delivery through the framework
//! costs each subscriber a wake of its connection task, a serialisation, a
//! frame through the pipe and a read by a WebSocket endpoint whose read
//! buffer the relay cannot size (see [`READ_BUFFER`]). The gate does with a
//! live event what the framework does: a subscription begins when its stored
//! answer ends, no connection hears a live event while the framework is
//! answering it, and one that falls a whole feed behind has its subscriptions
//! closed.
//!
//! This module imports nothing of the framework: it sees a stream to hand
//! over, which is what keeps the framework behind its one adapter.

use std::collections::{HashMap, HashSet, VecDeque};
use std::future::Future;
use std::sync::Arc;

use futures_util::{SinkExt, StreamExt};
use nostr::event::Event;
use nostr::filter::{Filter, MatchEventOptions};
use nostr::message::ClientMessage;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream};
use tokio::sync::broadcast;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};

use crate::connector::EdgeSlot;
use crate::document::write_refusal;
use crate::{Carriage, RelayError, Store};

/// The most subscriptions one connection holds. Replacing one is not another.
pub(crate) const MAX_SUBSCRIPTIONS: usize = 20;
/// The most filters one REQ carries.
pub(crate) const MAX_FILTERS: usize = 10;

/// The most stored events any one filter is answered with, and the number a
/// filter that names no `limit` is answered with. One number: the framework is
/// built with it, the gate's own stored answers keep to it and the Relay
/// Information Document states it as `max_limit` and `default_limit`.
pub(crate) const MAX_LIMIT: usize = 500;

/// The framework's own message ceiling (5 MiB), so the gate is not the
/// smaller limit.
const MAX_MESSAGE: usize = 5 * 1024 * 1024;
/// How much the pipe to the framework buffers each way.
const PIPE_BUFFER: usize = 64 * 1024;
/// The read buffer of each of the gate's two WebSocket endpoints per
/// connection: the one facing the client and the near end of the pipe.
///
/// tungstenite allocates this buffer when an endpoint is created and zero-fills
/// it on the first read, so at its default of 128 KiB every page is resident
/// for the life of the connection. Two of them, with the framework's own third,
/// made an idle connection cost about 415 KiB (821 MiB with 2000 idle
/// subscribers, against 179 MiB for the TypeScript image). At 4 KiB the two gate
/// buffers are gone and an idle connection costs about 160 KiB (322 MiB at
/// 2000, 87 MiB at 500). A message larger than the buffer still passes:
/// tungstenite grows the buffer to the frame it is reading.
const READ_BUFFER: usize = 4 * 1024;
/// How many live events may wait for a connection that is not taking them,
/// before it has missed one. The framework's own figure.
const LIVE_BUFFER: usize = 1024;
/// What a subscription is closed with when its connection missed live events:
/// the framework's words.
const CLOSED_OVERFLOW: &str =
    "error: live event buffer overflow; resubscribe to recover stored events";
/// The reason a connection past the cap is closed with.
const CLOSE_REASON_FULL: &str = "max connections reached";

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

/// What the gate does with one message from a client.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Pass this text to the framework.
    Forward(String),
    /// Answer with this `NOTICE`; the framework never hears the message.
    Refuse(String),
    /// Answer with this frame; the framework never hears the message.
    Answer(String),
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
            json: serde_json::to_string(event)
                .expect("an event is strings, numbers and lists, which always serialize"),
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
    fn listen(&self) -> broadcast::Receiver<Arc<LiveEvent>> {
        self.0.subscribe()
    }
}

/// The values a filter's `#ab`-style keys accept, by key without the `#`.
type MultiLetterKeys = Vec<(String, HashSet<String>)>;

/// Remove from `filter` the tag keys the framework ignores, and return them.
/// `None` when one of them is not a list, or `filter` is not an object.
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

/// One filter of a subscription: what the framework understands of it, and
/// the tag keys it does not.
#[derive(Debug)]
struct Wanted {
    base: Filter,
    /// `#ab`-style keys with the values they accept.
    multi: MultiLetterKeys,
}

impl Wanted {
    fn matches(&self, event: &Event) -> bool {
        self.base.match_event(event, MatchEventOptions::new())
            && self.multi.iter().all(|(name, values)| {
                event.tags.iter().any(|tag| {
                    tag.as_slice().first() == Some(name)
                        && tag.content().is_some_and(|value| values.contains(value))
                })
            })
    }

    /// The filter as the store is asked: with the limit the framework would
    /// have applied.
    fn query(&self) -> Filter {
        let mut filter = self.base.clone();
        let requested = filter
            .limit
            .unwrap_or_else(|| filter.ids.as_ref().map_or(MAX_LIMIT, |ids| ids.len()));
        filter.limit = Some(requested.min(MAX_LIMIT));
        filter
    }
}

/// A subscription the framework took: the gate delivers its live events.
#[derive(Debug)]
struct Subscription {
    filters: Vec<Wanted>,
    /// How each of its live frames begins: `["EVENT","<id>",`.
    head: String,
}

/// A message the framework has heard and not finished answering. It answers
/// one at a time, in the order it heard them, so what it sends belongs to
/// the oldest of these.
#[derive(Debug)]
enum Asked {
    /// A `REQ`: stored events, then `EOSE`; or `CLOSED` and a reason.
    Request {
        id: String,
        /// What it becomes at `EOSE`, which is when the framework takes a
        /// subscription. `None` once the client has closed it.
        subscription: Option<Subscription>,
        /// The gate gives its stored answer: the stored events the framework
        /// sends are held back, and at its `EOSE` the store's are sent.
        answered_here: bool,
    },
    /// A `COUNT`: a `COUNT`; or `CLOSED` and a reason.
    Count { id: String },
}

impl Asked {
    fn id(&self) -> &str {
        match self {
            Self::Request { id, .. } | Self::Count { id } => id,
        }
    }
}

/// What the gate does with one message from the framework.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Relayed {
    /// Pass it to the client.
    Pass,
    /// The client never hears it.
    Drop,
    /// The framework's stored answer to this subscription is over: send the
    /// store's ([`Gate::stored_queries`], [`Gate::stored_frames`]) instead.
    StoredDue(String),
}

/// One connection's view of its own subscriptions.
#[derive(Debug, Default)]
pub(crate) struct Gate {
    open: HashSet<String>,
    refusal: Refusal,
    /// Whether an expired event is still served, which the framework will
    /// not do for a stored answer.
    serves_expired: bool,
    subscriptions: HashMap<String, Subscription>,
    /// What the framework has heard and not finished answering, oldest first.
    awaiting: VecDeque<Asked>,
}

impl Gate {
    /// What to do with `text`, a message the client sent.
    pub(crate) fn client_sent(&mut self, text: &str) -> Verdict {
        let mut items = match serde_json::from_str::<Value>(text) {
            Ok(Value::Array(items)) => items,
            Ok(_) => {
                return Verdict::Refuse(
                    "error: invalid message format, expected JSON array".to_string(),
                );
            }
            Err(_) => return Verdict::Refuse("error: invalid JSON".to_string()),
        };
        match items.first().and_then(Value::as_str) {
            Some("CLOSE") => {
                if let Some(id) = items.get(1).and_then(Value::as_str) {
                    self.open.remove(id);
                    self.subscriptions.remove(id);
                    // The framework still answers a request it has heard, and
                    // closes it straight after: it is never live.
                    for asked in &mut self.awaiting {
                        if let Asked::Request {
                            id: asked,
                            subscription,
                            answered_here,
                        } = asked
                            && asked == id
                        {
                            *subscription = None;
                            *answered_here = false;
                        }
                    }
                }
                Verdict::Forward(text.to_string())
            }
            Some("COUNT") => {
                if let Ok(ClientMessage::Count {
                    subscription_id, ..
                }) = ClientMessage::from_json(text)
                {
                    self.awaiting.push_back(Asked::Count {
                        id: subscription_id.to_string(),
                    });
                }
                Verdict::Forward(text.to_string())
            }
            Some("REQ") => {
                let Some(id) = items
                    .get(1)
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty())
                    .map(str::to_string)
                else {
                    return Verdict::Refuse("error: invalid subscription id".to_string());
                };
                if !self.open.contains(&id) && self.open.len() >= MAX_SUBSCRIPTIONS {
                    return Verdict::Refuse("error: too many subscriptions".to_string());
                }
                if items.len() - 2 > MAX_FILTERS {
                    return Verdict::Refuse("error: too many filters".to_string());
                }
                self.open.insert(id.clone());
                let mut changed = false;
                for filter in items.iter_mut().skip(2) {
                    changed |= keep_whole_values(filter);
                }
                let whole = if changed {
                    Value::Array(items.clone()).to_string()
                } else {
                    text.to_string()
                };
                Verdict::Forward(self.ask(&id, items, whole))
            }
            Some("EVENT") => {
                let id = items
                    .get(1)
                    .and_then(|event| event.get("id"))
                    .cloned()
                    .unwrap_or(Value::Null);
                Verdict::Answer(json!(["OK", id, false, self.refusal.words()]).to_string())
            }
            Some("AUTH" | "NEG-OPEN" | "NEG-MSG" | "NEG-CLOSE") => {
                Verdict::Forward(text.to_string())
            }
            Some(other) => Verdict::Refuse(format!("error: unknown message type: {other}")),
            None => Verdict::Refuse("error: unknown message type: ".to_string()),
        }
    }

    /// Note the request `items` for `id`, and return what the framework is to
    /// hear of it: `whole`, the request as the client sent it, or the same
    /// without the tag keys the framework ignores, which the gate applies.
    fn ask(&mut self, id: &str, mut items: Vec<Value>, whole: String) -> String {
        let multi: Option<Vec<MultiLetterKeys>> = items
            .iter_mut()
            .skip(2)
            .map(take_multi_letter_keys)
            .collect();
        // A key that is not a list is left for the framework to judge: it
        // hears every filter as the client sent it, and gives the answer.
        let (multi, shaped) = match multi {
            Some(multi) => (multi, true),
            None => (Vec::new(), false),
        };
        let post_filter = multi.iter().any(|keys| !keys.is_empty());
        let heard = if post_filter {
            Value::Array(items).to_string()
        } else {
            whole.clone()
        };
        // The framework's own parser, on the text it will read: what parses
        // here it answers with `EOSE` or `CLOSED`, and what does not it
        // refuses in its own words, hearing the whole request.
        let Ok(ClientMessage::Req { filters, .. }) = ClientMessage::from_json(&heard) else {
            return whole;
        };
        let mut multi = multi.into_iter();
        let filters = filters
            .into_iter()
            .map(|base| Wanted {
                base: base.into_owned(),
                multi: multi.next().unwrap_or_default(),
            })
            .collect();
        self.awaiting.push_back(Asked::Request {
            id: id.to_string(),
            subscription: Some(Subscription {
                filters,
                head: format!("[\"EVENT\",{},", Value::from(id)),
            }),
            answered_here: shaped && (post_filter || self.serves_expired),
        });
        heard
    }

    /// What to do with `text`, a message the framework sent. A subscription
    /// it closed with a reason is no longer open, whoever closed it; a
    /// `CLOSED` with no reason is dropped and the subscription stays open.
    pub(crate) fn relay_sent(&mut self, text: &str) -> Relayed {
        // The framework is told of no new event, so every event it sends is
        // a stored one, in answer to the oldest request. Not parsed.
        if text.starts_with("[\"EVENT\"") {
            return match self.awaiting.front() {
                Some(Asked::Request {
                    answered_here: true,
                    ..
                }) => Relayed::Drop,
                _ => Relayed::Pass,
            };
        }
        if !["[\"EOSE\"", "[\"CLOSED\"", "[\"COUNT\""]
            .iter()
            .any(|head| text.starts_with(head))
        {
            return Relayed::Pass;
        }
        let Ok(Value::Array(items)) = serde_json::from_str::<Value>(text) else {
            return Relayed::Pass;
        };
        let (Some(kind), Some(id)) = (
            items.first().and_then(Value::as_str),
            items.get(1).and_then(Value::as_str),
        ) else {
            return Relayed::Pass;
        };
        let oldest = self.awaiting.front().filter(|asked| asked.id() == id);
        match (kind, oldest) {
            ("CLOSED", oldest) => {
                let reason = items.get(2).and_then(Value::as_str).unwrap_or_default();
                if reason.is_empty() {
                    // Not a refusal: the framework ends a by-`ids`
                    // subscription once it has returned as many events as
                    // ids. The TypeScript relay leaves such a subscription
                    // open, so the client never hears this, the place stays
                    // taken and the gate goes on delivering to it.
                    return Relayed::Drop;
                }
                self.open.remove(id);
                if oldest.is_some() {
                    // Refused. A subscription it would have replaced stands,
                    // as it does in the framework.
                    self.awaiting.pop_front();
                }
                Relayed::Pass
            }
            ("COUNT", Some(Asked::Count { .. })) => {
                self.awaiting.pop_front();
                Relayed::Pass
            }
            ("EOSE", Some(Asked::Request { .. })) => {
                let Some(Asked::Request {
                    id,
                    subscription,
                    answered_here,
                }) = self.awaiting.pop_front()
                else {
                    return Relayed::Pass;
                };
                if let Some(subscription) = subscription {
                    self.subscriptions.insert(id.clone(), subscription);
                }
                if answered_here {
                    Relayed::StoredDue(id)
                } else {
                    Relayed::Pass
                }
            }
            _ => Relayed::Pass,
        }
    }

    /// Whether this connection takes live events now. It takes none while
    /// the framework is still answering a request: an event that arrives
    /// during a stored answer is delivered after it, never inside it.
    pub(crate) fn hears_live(&self) -> bool {
        self.awaiting.is_empty()
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
            .map(|subscription| format!("{}{}]", subscription.head, live.json))
    }

    /// The connection missed live events, which cannot be matched any more:
    /// every subscription ends rather than carry on with a gap in it. Their
    /// ids, for the client and the framework to be told.
    pub(crate) fn overflowed(&mut self) -> Vec<String> {
        let ids: Vec<String> = self.subscriptions.drain().map(|(id, _)| id).collect();
        for id in &ids {
            self.open.remove(id);
        }
        ids
    }

    /// The questions to put to the store for `id`'s stored answer.
    pub(crate) fn stored_queries(&self, id: &str) -> Vec<Filter> {
        self.subscriptions
            .get(id)
            .map(|subscription| subscription.filters.iter().map(Wanted::query).collect())
            .unwrap_or_default()
    }

    /// The frames that answer `id`'s stored phase, from what the store found
    /// for each filter: every event once, newest first and the lower id
    /// first among equals, and then `EOSE`. A filter's `limit` is applied by
    /// the store before the tag keys the framework ignores are, so such a
    /// filter can be answered with fewer events than its limit.
    pub(crate) fn stored_frames(&self, id: &str, found: Vec<Vec<Event>>) -> Vec<String> {
        let mut events: Vec<Event> = Vec::new();
        let mut seen = HashSet::new();
        if let Some(subscription) = self.subscriptions.get(id) {
            for event in found.into_iter().flatten() {
                if seen.insert(event.id) && subscription.filters.iter().any(|f| f.matches(&event)) {
                    events.push(event);
                }
            }
        }
        events.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
        events
            .iter()
            .map(|event| json!(["EVENT", id, event]).to_string())
            .chain(std::iter::once(json!(["EOSE", id]).to_string()))
            .collect()
    }

    /// Stop shaping `id`: the store could not answer it.
    pub(crate) fn forget(&mut self, id: &str) {
        self.open.remove(id);
        self.subscriptions.remove(id);
    }
}

/// Drop from `filter`'s `ids` and `authors` every entry that is not a whole
/// 64-character hex value. An entry left empty matches nothing, which is what
/// a prefix does on a relay that matches exactly. Whether anything changed.
fn keep_whole_values(filter: &mut Value) -> bool {
    let mut changed = false;
    for key in ["ids", "authors"] {
        if let Some(Value::Array(values)) = filter.get_mut(key) {
            let before = values.len();
            values.retain(|value| {
                value.as_str().is_some_and(|hex| {
                    hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())
                })
            });
            changed |= values.len() != before;
        }
    }
    changed
}

/// Close a connection the relay has no room for: 1013, "try again later".
pub(crate) async fn refuse_full<S>(client: S) -> Result<(), RelayError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut client = WebSocketStream::from_raw_socket(client, Role::Server, None).await;
    // The peer may already be gone; there is no one left to tell.
    let _ = client
        .close(Some(CloseFrame {
            code: CloseCode::Again,
            reason: CLOSE_REASON_FULL.into(),
        }))
        .await;
    Ok(())
}

/// The frames that answer `id`'s stored phase from the store, or, when the
/// store cannot be read, the `CLOSED` the client is told instead.
async fn stored_answer(gate: &mut Gate, store: &Store, id: &str) -> Result<Vec<String>, String> {
    let mut found = Vec::new();
    for filter in gate.stored_queries(id) {
        match store.query(filter).await {
            Ok(events) => found.push(events),
            Err(error) => {
                eprintln!("read: subscription {id} could not be answered from the store: {error}");
                gate.forget(id);
                return Err(json!(["CLOSED", id, "error: the store could not be read"]).to_string());
            }
        }
    }
    Ok(gate.stored_frames(id, found))
}

/// Send each of `frames` to the client, in order, until one cannot be sent.
async fn send_all<S>(
    client: &mut WebSocketStream<S>,
    frames: impl IntoIterator<Item = String>,
) -> Result<(), tokio_tungstenite::tungstenite::Error>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    for frame in frames {
        client.send(Message::text(frame)).await?;
    }
    Ok(())
}

/// The configuration both of the gate's endpoints are built from.
fn socket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .read_buffer_size(READ_BUFFER)
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE))
}

/// Build the gate's two endpoints for one connection from `socket_config`:
/// the one facing `client` and the one on `near`, the near end of the pipe.
async fn endpoints<S>(
    client: S,
    near: DuplexStream,
) -> (WebSocketStream<S>, WebSocketStream<DuplexStream>)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let config = socket_config();
    let client = WebSocketStream::from_raw_socket(client, Role::Server, Some(config)).await;
    let near = WebSocketStream::from_raw_socket(near, Role::Client, Some(config)).await;
    (client, near)
}

/// Serve `client`, a connection already upgraded to WebSocket, until either
/// side closes it. `framework` is handed the far end of the pipe the framework
/// speaks on; `refusal` is what an `EVENT` is answered with, and `live` is
/// the feed its subscriptions are delivered from.
pub(crate) async fn through<S, F, Fut>(
    client: S,
    refusal: Refusal,
    store: Store,
    live: &LiveFeed,
    framework: F,
) -> Result<(), RelayError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnOnce(DuplexStream) -> Fut,
    Fut: Future<Output = Result<(), RelayError>> + Send + 'static,
{
    // Before anything is asked: no event published from here on is missed.
    let mut live = live.listen();
    let (near, far) = tokio::io::duplex(PIPE_BUFFER);
    let framework = tokio::spawn(framework(far));
    let (mut client, mut inner) = endpoints(client, near).await;
    let mut gate = Gate {
        serves_expired: !store.enforces_expiration(),
        refusal,
        ..Gate::default()
    };

    let ended = loop {
        tokio::select! {
            frame = client.next() => match frame {
                Some(Ok(Message::Text(text))) => match gate.client_sent(&text) {
                    Verdict::Forward(text) => {
                        if inner.send(Message::text(text)).await.is_err() {
                            break Ok(());
                        }
                    }
                    Verdict::Refuse(notice) => {
                        let frame = json!(["NOTICE", notice]).to_string();
                        if let Err(error) = send_all(&mut client, [frame]).await {
                            break Err(error);
                        }
                    }
                    Verdict::Answer(frame) => {
                        if let Err(error) = send_all(&mut client, [frame]).await {
                            break Err(error);
                        }
                    }
                },
                Some(Ok(Message::Binary(bytes))) => {
                    if inner.send(Message::Binary(bytes)).await.is_err() {
                        break Ok(());
                    }
                }
                Some(Ok(Message::Close(_))) | None => break Ok(()),
                // Pings are answered by the WebSocket layer itself.
                Some(Ok(_)) => {}
                Some(Err(error)) => break Err(error),
            },
            frame = inner.next() => match frame {
                Some(Ok(Message::Text(text))) => {
                    let frames = match gate.relay_sent(&text) {
                        Relayed::Pass => vec![text.to_string()],
                        Relayed::Drop => Vec::new(),
                        Relayed::StoredDue(id) => match stored_answer(&mut gate, &store, &id).await {
                            Ok(frames) => frames,
                            // The client is told it is closed, so the framework
                            // closes it too.
                            Err(closed) => {
                                let close = json!(["CLOSE", id]).to_string();
                                if inner.send(Message::text(close)).await.is_err() {
                                    break Ok(());
                                }
                                vec![closed]
                            }
                        },
                    };
                    if let Err(error) = send_all(&mut client, frames).await {
                        break Err(error);
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break Ok(()),
                Some(Ok(_)) => {}
            },
            event = live.recv(), if gate.hears_live() => match event {
                Ok(event) => {
                    if let Err(error) = send_all(&mut client, gate.live_frames(&event)).await {
                        break Err(error);
                    }
                }
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    live = live.resubscribe();
                    let ended = gate.overflowed();
                    // The framework closes them too, as the client is told.
                    let mut heard = true;
                    for id in &ended {
                        let close = json!(["CLOSE", id]).to_string();
                        heard &= inner.send(Message::text(close)).await.is_ok();
                    }
                    if !heard {
                        break Ok(());
                    }
                    let closed: Vec<String> = ended
                        .iter()
                        .map(|id| json!(["CLOSED", id, CLOSED_OVERFLOW]).to_string())
                        .collect();
                    if let Err(error) = send_all(&mut client, closed).await {
                        break Err(error);
                    }
                }
                // The relay is going: every clone of the feed is dropped.
                Err(broadcast::error::RecvError::Closed) => break Ok(()),
            },
        }
    };

    // Say goodbye to a client that is still there, then let the framework go.
    let _ = client.close(None).await;
    if framework.is_finished() {
        framework
            .await
            .map_err(|error| RelayError::ReadSide(error.to_string()))??;
    } else {
        framework.abort();
    }
    use tokio_tungstenite::tungstenite::Error::{AlreadyClosed, ConnectionClosed};
    match ended {
        Err(AlreadyClosed | ConnectionClosed) | Ok(()) => Ok(()),
        Err(error) => Err(RelayError::ReadSide(error.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn the_gates_endpoints_do_not_use_the_default_read_buffer() {
        let (client, _) = tokio::io::duplex(PIPE_BUFFER);
        let (near, _) = tokio::io::duplex(PIPE_BUFFER);
        let (client, near) = endpoints(client, near).await;
        for config in [client.get_config(), near.get_config()] {
            assert_eq!(config.read_buffer_size, READ_BUFFER);
            assert!(config.read_buffer_size < WebSocketConfig::default().read_buffer_size);
            assert_eq!(config.max_message_size, Some(MAX_MESSAGE));
            assert_eq!(config.max_frame_size, Some(MAX_MESSAGE));
        }
    }

    #[tokio::test]
    async fn a_message_larger_than_the_read_buffer_passes_both_ways() {
        use futures_util::{SinkExt, StreamExt};
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("relay.db")).unwrap();
        let (client_end, gate_end) = tokio::io::duplex(PIPE_BUFFER);
        let gate = tokio::spawn(async move {
            through(
                gate_end,
                Refusal::default(),
                store,
                &LiveFeed::new(),
                |pipe| async move {
                    // The framework end, on tungstenite's defaults like the real
                    // one: send every text back as it came.
                    let mut socket =
                        WebSocketStream::from_raw_socket(pipe, Role::Server, None).await;
                    while let Some(Ok(message)) = socket.next().await {
                        if message.is_text() && socket.send(message).await.is_err() {
                            break;
                        }
                    }
                    Ok(())
                },
            )
            .await
        });
        let mut client =
            WebSocketStream::from_raw_socket(client_end, Role::Client, Some(socket_config())).await;
        // A message the gate forwards: what is not one is answered here and
        // never crosses the pipe.
        let text = json!(["NEG-MSG", "n", "x".repeat(MAX_MESSAGE / 2)]).to_string();
        assert!(text.len() > READ_BUFFER);
        client.send(Message::text(text.clone())).await.unwrap();
        match client.next().await {
            Some(Ok(Message::Text(echoed))) => assert_eq!(echoed.as_str(), text),
            other => panic!("expected the whole message back, got {other:?}"),
        }
        let _ = client.close(None).await;
        let _ = gate.await;
    }

    fn forwarded(text: &str) -> Verdict {
        Verdict::Forward(text.to_string())
    }

    #[test]
    fn a_request_within_the_limits_passes_untouched() {
        let mut gate = Gate::default();
        let text = r#"["REQ","a",{"kinds":[1]},{"kinds":[7]}]"#;
        assert_eq!(gate.client_sent(text), forwarded(text));
    }

    #[test]
    fn every_event_is_refused_by_the_gate_whatever_it_is() {
        let mut gate = Gate::default();
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
                gate.client_sent(text),
                Verdict::Answer(json!(["OK", id, false, refusal]).to_string()),
                "{text}"
            );
        }
    }

    #[test]
    fn an_empty_subscription_id_is_a_notice() {
        let mut gate = Gate::default();
        assert_eq!(
            gate.client_sent(r#"["REQ","",{}]"#),
            Verdict::Refuse("error: invalid subscription id".to_string())
        );
    }

    #[test]
    fn what_is_not_a_message_is_named_in_a_notice_of_its_own() {
        let mut gate = Gate::default();
        for (text, notice) in [
            ("{not json", "error: invalid JSON"),
            (
                r#"{"a":1}"#,
                "error: invalid message format, expected JSON array",
            ),
            (r#"["BOGUS","x"]"#, "error: unknown message type: BOGUS"),
            (r#"["REQ",7,{}]"#, "error: invalid subscription id"),
        ] {
            assert_eq!(
                gate.client_sent(text),
                Verdict::Refuse(notice.to_string()),
                "{text}"
            );
        }
    }

    #[test]
    fn the_messages_the_framework_answers_are_forwarded() {
        let mut gate = Gate::default();
        for text in [
            r#"["COUNT","c",{}]"#,
            r#"["AUTH",{}]"#,
            r#"["NEG-OPEN","n",{},"00"]"#,
            r#"["NEG-MSG","n","00"]"#,
            r#"["NEG-CLOSE","n"]"#,
            r#"["CLOSE","a"]"#,
        ] {
            assert_eq!(gate.client_sent(text), forwarded(text), "{text}");
        }
    }

    #[test]
    fn a_closed_without_a_reason_is_never_heard_and_frees_nothing() {
        let mut gate = Gate::default();
        for i in 0..MAX_SUBSCRIPTIONS {
            gate.client_sent(&format!(r#"["REQ","s{i}",{{}}]"#));
        }
        assert_eq!(gate.relay_sent(r#"["CLOSED","s0",""]"#), Relayed::Drop);
        assert_eq!(
            gate.client_sent(r#"["REQ","more",{}]"#),
            Verdict::Refuse("error: too many subscriptions".to_string())
        );
        assert_eq!(
            gate.relay_sent(r#"["CLOSED","s1","error: x"]"#),
            Relayed::Pass
        );
        let text = r#"["REQ","more",{}]"#;
        assert_eq!(gate.client_sent(text), forwarded(text));
        gate.client_sent(r#"["CLOSE","s0"]"#);
        assert_eq!(gate.open.len(), MAX_SUBSCRIPTIONS - 1);
    }

    #[test]
    fn the_subscription_after_the_limit_is_a_notice_but_replacing_one_is_not() {
        let mut gate = Gate::default();
        for i in 0..MAX_SUBSCRIPTIONS {
            let text = format!(r#"["REQ","s{i}",{{}}]"#);
            assert_eq!(gate.client_sent(&text), Verdict::Forward(text));
        }
        assert_eq!(
            gate.client_sent(r#"["REQ","more",{}]"#),
            Verdict::Refuse("error: too many subscriptions".to_string())
        );
        let replace = r#"["REQ","s0",{}]"#;
        assert_eq!(gate.client_sent(replace), forwarded(replace));
    }

    #[test]
    fn a_closed_subscription_frees_its_place_whoever_closed_it() {
        let mut gate = Gate::default();
        for i in 0..MAX_SUBSCRIPTIONS {
            gate.client_sent(&format!(r#"["REQ","s{i}",{{}}]"#));
        }
        gate.client_sent(r#"["CLOSE","s0"]"#);
        gate.relay_sent(r#"["CLOSED","s1","error: live event buffer overflow"]"#);
        for id in ["a", "b"] {
            let text = format!(r#"["REQ","{id}",{{}}]"#);
            assert_eq!(gate.client_sent(&text), Verdict::Forward(text));
        }
        assert_eq!(
            gate.client_sent(r#"["REQ","c",{}]"#),
            Verdict::Refuse("error: too many subscriptions".to_string())
        );
    }

    #[test]
    fn a_request_with_too_many_filters_is_a_notice_and_one_at_the_limit_is_not() {
        let mut gate = Gate::default();
        let request = |filters: usize| {
            let filters = vec!["{}"; filters].join(",");
            format!(r#"["REQ","f",{filters}]"#)
        };
        assert_eq!(
            gate.client_sent(&request(MAX_FILTERS + 1)),
            Verdict::Refuse("error: too many filters".to_string())
        );
        let at_limit = request(MAX_FILTERS);
        assert_eq!(gate.client_sent(&at_limit), Verdict::Forward(at_limit));
    }

    #[test]
    fn a_refused_request_opens_nothing() {
        let mut gate = Gate::default();
        gate.client_sent(&format!(
            r#"["REQ","f",{}]"#,
            ["{}"; MAX_FILTERS + 1].join(",")
        ));
        assert!(gate.open.is_empty());
    }

    #[test]
    fn an_id_or_author_that_is_not_a_whole_value_matches_nothing() {
        let mut gate = Gate::default();
        let whole = "ab".repeat(32);
        let text = format!(r#"["REQ","p",{{"ids":["abcd","{whole}"],"authors":["abcd"]}}]"#);
        let Verdict::Forward(sent) = gate.client_sent(&text) else {
            panic!("a request is forwarded");
        };
        let sent: Value = serde_json::from_str(&sent).expect("the gate sends JSON");
        assert_eq!(sent[2], json!({ "ids": [whole], "authors": [] }));
    }

    #[test]
    fn a_request_the_framework_can_answer_whole_keeps_the_frameworks_answer() {
        let mut gate = Gate::default();
        let text = r##"["REQ","a",{"kinds":[1],"#t":["x"]}]"##;
        assert_eq!(gate.client_sent(text), forwarded(text));
        assert_eq!(gate.relay_sent(r#"["EOSE","a"]"#), Relayed::Pass);
    }

    #[test]
    fn a_multi_letter_key_is_hidden_from_the_framework_and_its_eose_is_held_for_the_store() {
        let mut gate = Gate::default();
        let Verdict::Forward(sent) = gate.client_sent(r##"["REQ","a",{"kinds":[1],"#ab":["x"]}]"##)
        else {
            panic!("forwarded");
        };
        assert_eq!(sent, r#"["REQ","a",{"kinds":[1]}]"#);
        let event = r#"["EVENT","a",{}]"#;
        assert_eq!(gate.relay_sent(event), Relayed::Drop);
        assert_eq!(
            gate.relay_sent(r#"["EOSE","a"]"#),
            Relayed::StoredDue("a".to_string())
        );
        assert_eq!(gate.stored_queries("a").len(), 1);
    }

    #[test]
    fn while_expired_events_are_served_every_request_is_answered_from_the_store() {
        let mut gate = Gate {
            serves_expired: true,
            ..Gate::default()
        };
        let text = r#"["REQ","a",{"kinds":[1]}]"#;
        assert_eq!(gate.client_sent(text), forwarded(text));
        assert_eq!(
            gate.relay_sent(r#"["EOSE","a"]"#),
            Relayed::StoredDue("a".to_string())
        );
        let frames = gate.stored_frames("a", vec![]);
        assert_eq!(frames, vec![r#"["EOSE","a"]"#.to_string()]);
        assert_eq!(gate.relay_sent(r#"["EVENT","a",{}]"#), Relayed::Pass);
    }

    #[test]
    fn a_request_the_framework_must_refuse_reaches_it_with_every_filter_whole() {
        let mut gate = Gate::default();
        let whole = "ab".repeat(32);
        let text =
            format!(r##"["REQ","a",{{"#ab":["x"],"ids":["abcd","{whole}"]}},{{"kinds":"x"}}]"##);
        let Verdict::Forward(sent) = gate.client_sent(&text) else {
            panic!("forwarded");
        };
        let sent: Value = serde_json::from_str(&sent).expect("the gate sends JSON");
        assert_eq!(sent[2], json!({ "#ab": ["x"], "ids": [whole] }));
        assert_eq!(sent[3], json!({ "kinds": "x" }));
        assert!(gate.stored_queries("a").is_empty());
    }

    #[test]
    fn a_filter_without_a_limit_is_asked_with_the_frameworks_default() {
        let mut gate = Gate {
            serves_expired: true,
            ..Gate::default()
        };
        gate.client_sent(r#"["REQ","a",{"kinds":[1]},{"limit":3},{"limit":9999}]"#);
        gate.relay_sent(r#"["EOSE","a"]"#);
        let limits: Vec<_> = gate
            .stored_queries("a")
            .iter()
            .map(|filter| filter.limit)
            .collect();
        assert_eq!(limits, vec![Some(MAX_LIMIT), Some(3), Some(MAX_LIMIT)]);
    }

    /// A signed event of `kind` carrying `tags`.
    fn event(kind: u16, tags: &[&[&str]]) -> Event {
        use nostr::event::{EventBuilder, FinalizeEvent, Kind, Tag};
        let tags = tags
            .iter()
            .map(|tag| Tag::parse(tag.iter().copied()).expect("a non-empty tag parses"));
        EventBuilder::new(Kind::from(kind), "live")
            .tags(tags)
            .finalize(&nostr::key::Keys::generate())
            .expect("a generated key signs an event")
    }

    /// A gate whose every request in `requests` the framework has answered.
    fn subscribed(requests: &[&str]) -> Gate {
        let mut gate = Gate::default();
        for text in requests {
            gate.client_sent(text);
        }
        while let Some(asked) = gate.awaiting.front() {
            let eose = json!(["EOSE", asked.id()]).to_string();
            gate.relay_sent(&eose);
        }
        gate
    }

    fn frames(gate: &Gate, event: &Event) -> Vec<String> {
        let mut frames: Vec<String> = gate.live_frames(&LiveEvent::new(event)).collect();
        frames.sort();
        frames
    }

    #[test]
    fn a_live_event_is_one_frame_for_each_subscription_it_matches() {
        let gate = subscribed(&[
            r#"["REQ","a",{"kinds":[1]}]"#,
            r#"["REQ","b",{"kinds":[7]},{"kinds":[1]},{}]"#,
            r#"["REQ","c",{"kinds":[7]}]"#,
        ]);
        let event = event(1, &[]);
        let json = serde_json::to_string(&event).expect("JSON");
        assert_eq!(
            frames(&gate, &event),
            vec![
                format!(r#"["EVENT","a",{json}]"#),
                format!(r#"["EVENT","b",{json}]"#)
            ]
        );
    }

    #[test]
    fn a_live_frame_quotes_its_subscription_id_as_json_does() {
        let gate = subscribed(&[r#"["REQ","a\"\\b",{}]"#]);
        let frame = frames(&gate, &event(1, &[])).remove(0);
        let frame: Value = serde_json::from_str(&frame).expect("a live frame is JSON");
        assert_eq!(frame[1], json!("a\"\\b"));
    }

    #[test]
    fn no_live_event_is_heard_while_the_framework_is_answering_a_request() {
        let mut gate = Gate::default();
        assert!(gate.hears_live());
        gate.client_sent(r#"["REQ","a",{}]"#);
        gate.client_sent(r#"["REQ","b",{}]"#);
        assert!(!gate.hears_live());
        // A stored event is not the end of an answer; `EOSE` and `CLOSED` are.
        gate.relay_sent(r#"["EVENT","a",{}]"#);
        gate.relay_sent(r#"["EOSE","a"]"#);
        assert!(!gate.hears_live(), "b is still being answered");
        gate.relay_sent(r#"["CLOSED","b","rate-limited: too many queries"]"#);
        assert!(gate.hears_live());
        let frames = frames(&gate, &event(1, &[]));
        assert_eq!(frames.len(), 1, "a is live and b was refused");
        assert!(frames[0].starts_with(r#"["EVENT","a","#));
    }

    #[test]
    fn a_request_the_framework_will_not_parse_is_no_subscription() {
        let mut gate = Gate::default();
        for text in [r#"["REQ","a"]"#, r#"["REQ","a",{"kinds":"x"}]"#] {
            assert_eq!(gate.client_sent(text), forwarded(text));
            assert!(gate.hears_live(), "nothing is awaited for {text}");
            assert!(frames(&gate, &event(1, &[])).is_empty());
        }
    }

    #[test]
    fn a_refused_request_leaves_the_subscription_it_would_have_replaced() {
        let mut gate = subscribed(&[r#"["REQ","a",{"kinds":[1]}]"#]);
        gate.client_sent(r#"["REQ","a",{"kinds":"x"}]"#);
        assert_eq!(frames(&gate, &event(1, &[])).len(), 1);
    }

    #[test]
    fn a_request_refused_with_closed_leaves_the_subscription_it_would_have_replaced() {
        let mut gate = subscribed(&[r#"["REQ","a",{"kinds":[1]}]"#]);
        gate.client_sent(r#"["REQ","a",{"kinds":[7]}]"#);
        gate.relay_sent(r#"["CLOSED","a","rate-limited: too many queries"]"#);
        assert!(gate.hears_live());
        assert_eq!(frames(&gate, &event(1, &[])).len(), 1);
        assert!(frames(&gate, &event(7, &[])).is_empty());
    }

    #[test]
    fn a_request_sent_behind_one_the_framework_closes_itself_is_still_live() {
        // The first names its event by id and is sent it, so the framework
        // ends it: `EOSE`, then `CLOSED` with no reason. The second, with
        // the same id, was already on its way.
        let mut gate = Gate::default();
        gate.client_sent(&format!(r#"["REQ","a",{{"ids":["{}"]}}]"#, "ab".repeat(32)));
        gate.client_sent(r#"["REQ","a",{"kinds":[1]}]"#);
        gate.relay_sent(r#"["EVENT","a",{}]"#);
        gate.relay_sent(r#"["EOSE","a"]"#);
        gate.relay_sent(r#"["CLOSED","a",""]"#);
        assert!(!gate.hears_live(), "the second is still being answered");
        gate.relay_sent(r#"["EOSE","a"]"#);
        assert!(gate.hears_live());
        assert_eq!(frames(&gate, &event(1, &[])).len(), 1);
    }

    #[test]
    fn a_request_closed_before_it_is_answered_never_becomes_live() {
        let mut gate = Gate::default();
        gate.client_sent(r##"["REQ","a",{"#ab":["x"]}]"##);
        gate.client_sent(r#"["CLOSE","a"]"#);
        // The framework answers what it had heard, and that is passed on.
        assert_eq!(gate.relay_sent(r#"["EVENT","a",{}]"#), Relayed::Pass);
        assert_eq!(gate.relay_sent(r#"["EOSE","a"]"#), Relayed::Pass);
        assert!(gate.hears_live());
        assert!(frames(&gate, &event(1, &[&["ab", "x"]])).is_empty());
    }

    #[test]
    fn a_count_is_awaited_and_its_answer_ends_no_request() {
        let mut gate = Gate::default();
        gate.client_sent(r#"["COUNT","a",{}]"#);
        gate.client_sent(r#"["REQ","a",{}]"#);
        assert!(!gate.hears_live());
        gate.relay_sent(r#"["CLOSED","a","rate-limited: too many queries"]"#);
        assert!(!gate.hears_live(), "that refused the count");
        gate.relay_sent(r#"["EOSE","a"]"#);
        assert!(gate.hears_live());
        assert_eq!(frames(&gate, &event(1, &[])).len(), 1);

        gate.client_sent(r#"["COUNT","c",{}]"#);
        gate.relay_sent(r#"["COUNT","c",{"count":0}]"#);
        assert!(gate.hears_live());
    }

    #[test]
    fn only_a_subscription_the_client_closed_hears_no_live_event() {
        let mut gate = subscribed(&[r#"["REQ","a",{}]"#, r#"["REQ","b",{}]"#]);
        gate.client_sent(r#"["CLOSE","a"]"#);
        assert_eq!(gate.relay_sent(r#"["CLOSED","b",""]"#), Relayed::Drop);
        let frames = frames(&gate, &event(1, &[]));
        assert_eq!(frames.len(), 1, "b was not closed by the client");
        assert!(frames[0].starts_with(r#"["EVENT","b","#));
    }

    #[test]
    fn a_multi_letter_key_is_applied_to_live_events() {
        let mut gate = Gate::default();
        gate.client_sent(r##"["REQ","a",{"kinds":[1],"#ab":["x"]}]"##);
        assert_eq!(
            gate.relay_sent(r#"["EOSE","a"]"#),
            Relayed::StoredDue("a".to_string())
        );
        assert_eq!(frames(&gate, &event(1, &[&["ab", "x"]])).len(), 1);
        assert!(frames(&gate, &event(1, &[&["ab", "y"]])).is_empty());
        assert!(frames(&gate, &event(7, &[&["ab", "x"]])).is_empty());
    }

    #[test]
    fn a_multi_letter_key_that_is_not_a_list_is_left_to_the_framework() {
        let mut gate = Gate::default();
        let text = r##"["REQ","a",{"kinds":[1],"#ab":"x"}]"##;
        assert_eq!(gate.client_sent(text), forwarded(text));
        // The framework ignores the key, so the subscription is live without it.
        assert_eq!(gate.relay_sent(r#"["EOSE","a"]"#), Relayed::Pass);
        assert_eq!(frames(&gate, &event(1, &[])).len(), 1);

        // And the stored answer stays the framework's, expired events or not.
        let mut gate = Gate {
            serves_expired: true,
            ..Gate::default()
        };
        gate.client_sent(text);
        assert_eq!(gate.relay_sent(r#"["EOSE","a"]"#), Relayed::Pass);
    }

    #[test]
    fn a_connection_that_missed_live_events_loses_every_subscription() {
        let mut gate = subscribed(&[r#"["REQ","a",{}]"#, r#"["REQ","b",{}]"#]);
        let mut ended = gate.overflowed();
        ended.sort();
        assert_eq!(ended, vec!["a".to_string(), "b".to_string()]);
        assert!(gate.open.is_empty());
        assert!(frames(&gate, &event(1, &[])).is_empty());
    }

    /// A connection through the gate to a fake framework: one that hears
    /// what the gate forwards and says `EOSE` only when the test lets it.
    struct Connection {
        client: WebSocketStream<DuplexStream>,
        live: LiveFeed,
        /// What the framework heard, frame by frame.
        heard: tokio::sync::mpsc::UnboundedReceiver<String>,
        /// Each id sent here is answered with `EOSE`.
        eose: tokio::sync::mpsc::UnboundedSender<&'static str>,
        _data: tempfile::TempDir,
    }

    impl Connection {
        async fn open() -> Self {
            let data = tempfile::tempdir().expect("a temp dir");
            let store = Store::open(&data.path().join("events.db")).expect("a new database opens");
            let live = LiveFeed::new();
            let (ours, theirs) = tokio::io::duplex(PIPE_BUFFER);
            let (hear, heard) = tokio::sync::mpsc::unbounded_channel();
            let (eose, mut answers) = tokio::sync::mpsc::unbounded_channel::<&'static str>();
            let feed = live.clone();
            tokio::spawn(async move {
                let framework = move |pipe| async move {
                    let mut pipe = WebSocketStream::from_raw_socket(pipe, Role::Server, None).await;
                    loop {
                        tokio::select! {
                            frame = pipe.next() => match frame {
                                Some(Ok(Message::Text(text))) => {
                                    let _ = hear.send(text.to_string());
                                }
                                _ => break,
                            },
                            Some(id) = answers.recv() => {
                                let frame = json!(["EOSE", id]).to_string();
                                if pipe.send(Message::text(frame)).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Ok(())
                };
                through(theirs, Refusal::default(), store, &feed, framework).await
            });
            let client = WebSocketStream::from_raw_socket(ours, Role::Client, None).await;
            Self {
                client,
                live,
                heard,
                eose,
                _data: data,
            }
        }

        async fn send(&mut self, text: &str) {
            self.client
                .send(Message::text(text))
                .await
                .expect("the gate is listening");
        }

        /// The next frame the client is sent, or `None` after a second of
        /// silence.
        async fn next(&mut self) -> Option<Value> {
            let wait = std::time::Duration::from_secs(1);
            match tokio::time::timeout(wait, self.client.next()).await {
                Ok(Some(Ok(Message::Text(text)))) => {
                    Some(serde_json::from_str(&text).expect("a relay frame is JSON"))
                }
                _ => None,
            }
        }

        /// The next frame the framework heard.
        async fn framework_heard(&mut self) -> Option<Value> {
            let wait = std::time::Duration::from_secs(1);
            let text = tokio::time::timeout(wait, self.heard.recv()).await.ok()??;
            Some(serde_json::from_str(&text).expect("the gate forwards JSON"))
        }
    }

    #[tokio::test]
    async fn an_event_that_arrives_during_a_stored_answer_is_delivered_after_it() {
        let mut connection = Connection::open().await;
        connection.send(r#"["REQ","a",{}]"#).await;
        assert_eq!(
            connection.framework_heard().await,
            Some(json!(["REQ", "a", {}]))
        );

        let event = event(1, &[]);
        connection.live.publish(&event);
        assert_eq!(connection.next().await, None, "the answer is not over");

        connection.eose.send("a").expect("the framework is there");
        assert_eq!(connection.next().await, Some(json!(["EOSE", "a"])));
        assert_eq!(connection.next().await, Some(json!(["EVENT", "a", event])));
    }

    #[tokio::test]
    async fn a_connection_a_whole_feed_behind_is_told_its_subscriptions_are_closed() {
        let mut connection = Connection::open().await;
        connection.send(r#"["REQ","a",{}]"#).await;
        connection.framework_heard().await;

        // One more than the feed holds, while the connection takes none.
        let event = event(1, &[]);
        for _ in 0..=LIVE_BUFFER {
            connection.live.publish(&event);
        }
        connection.eose.send("a").expect("the framework is there");

        assert_eq!(connection.next().await, Some(json!(["EOSE", "a"])));
        assert_eq!(
            connection.next().await,
            Some(json!(["CLOSED", "a", CLOSED_OVERFLOW]))
        );
        assert_eq!(connection.next().await, None, "and no event after it");
        assert_eq!(
            connection.framework_heard().await,
            Some(json!(["CLOSE", "a"])),
            "the framework is told too"
        );
    }

    #[test]
    fn a_closed_subscription_is_not_answered_from_the_store() {
        let mut gate = Gate {
            serves_expired: true,
            ..Gate::default()
        };
        gate.client_sent(r#"["REQ","a",{"kinds":[1]}]"#);
        gate.client_sent(r#"["CLOSE","a"]"#);
        assert!(gate.stored_queries("a").is_empty());
    }
}
