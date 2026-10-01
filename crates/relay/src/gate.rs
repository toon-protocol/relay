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
//! This module imports nothing of the framework: it sees a stream to hand
//! over, which is what keeps the framework behind its one adapter.

use std::collections::{HashMap, HashSet};
use std::future::Future;

use futures_util::{SinkExt, StreamExt};
use nostr::event::Event;
use nostr::filter::{Filter, MatchEventOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream};
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

/// What the framework gives a filter that names no `limit`, and the most it
/// answers any one filter with. The gate answers in its place for some
/// subscriptions and keeps to the same numbers.
const FRAMEWORK_LIMIT: usize = 500;

/// The framework's own message ceiling (5 MiB), so the gate is not the
/// smaller limit.
const MAX_MESSAGE: usize = 5 * 1024 * 1024;
/// How much the pipe to the framework buffers each way.
const PIPE_BUFFER: usize = 64 * 1024;
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
    Refuse(&'static str),
    /// Answer with this frame; the framework never hears the message.
    Answer(String),
}

/// One filter of a subscription the gate answers for: what the framework
/// understands of it, and the tag keys it does not.
#[derive(Debug)]
struct Wanted {
    base: Filter,
    /// `#ab`-style keys with the values they accept.
    multi: Vec<(String, HashSet<String>)>,
}

impl Wanted {
    /// Split `filter`, removing the keys the framework ignores from it.
    /// `None` when it is not a filter the framework would take either.
    fn take(filter: &mut Value) -> Option<Self> {
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
        let base = serde_json::from_value(filter.clone()).ok()?;
        Some(Self { base, multi })
    }

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
            .unwrap_or_else(|| filter.ids.as_ref().map_or(FRAMEWORK_LIMIT, |ids| ids.len()));
        filter.limit = Some(requested.min(FRAMEWORK_LIMIT));
        filter
    }
}

/// A subscription whose answer the gate shapes.
#[derive(Debug)]
struct Watched {
    filters: Vec<Wanted>,
    /// The framework has not yet said `EOSE`: its stored events are held back.
    stored_pending: bool,
    /// Some filter has a key the framework ignores, so each event it sends
    /// is checked.
    post_filter: bool,
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
    watched: HashMap<String, Watched>,
}

impl Gate {
    /// What to do with `text`, a message the client sent.
    pub(crate) fn client_sent(&mut self, text: &str) -> Verdict {
        let Ok(Value::Array(mut items)) = serde_json::from_str::<Value>(text) else {
            // Not a message: the framework says so in its own NOTICE.
            return Verdict::Forward(text.to_string());
        };
        match items.first().and_then(Value::as_str) {
            Some("CLOSE") => {
                if let Some(id) = items.get(1).and_then(Value::as_str) {
                    self.open.remove(id);
                    self.watched.remove(id);
                }
                Verdict::Forward(text.to_string())
            }
            Some("REQ") => {
                let Some(id) = items.get(1).and_then(Value::as_str).map(str::to_string) else {
                    return Verdict::Forward(text.to_string());
                };
                if id.is_empty() {
                    return Verdict::Refuse("error: invalid subscription id");
                }
                if !self.open.contains(&id) && self.open.len() >= MAX_SUBSCRIPTIONS {
                    return Verdict::Refuse("error: too many subscriptions");
                }
                if items.len() - 2 > MAX_FILTERS {
                    return Verdict::Refuse("error: too many filters");
                }
                self.open.insert(id.clone());
                let mut changed = false;
                for filter in items.iter_mut().skip(2) {
                    changed |= keep_whole_values(filter);
                }
                changed |= self.watch(&id, &mut items);
                Verdict::Forward(if changed {
                    Value::Array(items).to_string()
                } else {
                    text.to_string()
                })
            }
            Some("EVENT") => {
                let id = items
                    .get(1)
                    .and_then(|event| event.get("id"))
                    .cloned()
                    .unwrap_or(Value::Null);
                Verdict::Answer(json!(["OK", id, false, self.refusal.words()]).to_string())
            }
            _ => Verdict::Forward(text.to_string()),
        }
    }

    /// Start answering for `id` where the framework cannot: the filters in
    /// `items` (after the id) lose the keys it ignores. Whether any did.
    fn watch(&mut self, id: &str, items: &mut [Value]) -> bool {
        self.watched.remove(id);
        let mut filters = Vec::new();
        let mut stripped = false;
        for filter in items.iter_mut().skip(2) {
            let before = filter.clone();
            match Wanted::take(filter) {
                Some(wanted) => {
                    stripped |= *filter != before;
                    filters.push(wanted);
                }
                // The framework answers it in its own words.
                None => {
                    *filter = before;
                    return false;
                }
            }
        }
        let post_filter = filters.iter().any(|wanted| !wanted.multi.is_empty());
        if post_filter || self.serves_expired {
            self.watched.insert(
                id.to_string(),
                Watched {
                    filters,
                    stored_pending: true,
                    post_filter,
                },
            );
        }
        stripped
    }

    /// What to do with `text`, a message the framework sent. A subscription
    /// it closed is no longer open, whoever closed it.
    pub(crate) fn relay_sent(&mut self, text: &str) -> Relayed {
        let closed = text.starts_with("[\"CLOSED\"");
        if !closed && self.watched.is_empty() {
            return Relayed::Pass;
        }
        let Ok(Value::Array(items)) = serde_json::from_str::<Value>(text) else {
            return Relayed::Pass;
        };
        let id = items.get(1).and_then(Value::as_str);
        if closed {
            if let Some(id) = id {
                self.open.remove(id);
                self.watched.remove(id);
            }
            return Relayed::Pass;
        }
        let Some((id, watched)) = id.and_then(|id| self.watched.get_mut(id).map(|w| (id, w)))
        else {
            return Relayed::Pass;
        };
        match items.first().and_then(Value::as_str) {
            Some("EOSE") if watched.stored_pending => {
                watched.stored_pending = false;
                Relayed::StoredDue(id.to_string())
            }
            Some("EVENT") if watched.stored_pending => Relayed::Drop,
            Some("EVENT") if watched.post_filter => {
                let wanted = items
                    .get(2)
                    .and_then(|event| serde_json::from_value::<Event>(event.clone()).ok())
                    .is_some_and(|event| watched.filters.iter().any(|f| f.matches(&event)));
                if wanted { Relayed::Pass } else { Relayed::Drop }
            }
            _ => Relayed::Pass,
        }
    }

    /// The questions to put to the store for `id`'s stored answer.
    pub(crate) fn stored_queries(&self, id: &str) -> Vec<Filter> {
        self.watched
            .get(id)
            .map(|watched| watched.filters.iter().map(Wanted::query).collect())
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
        if let Some(watched) = self.watched.get(id) {
            for event in found.into_iter().flatten() {
                if seen.insert(event.id) && watched.filters.iter().any(|f| f.matches(&event)) {
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
        self.watched.remove(id);
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

/// The frames that answer `id`'s stored phase from the store.
async fn stored_answer(gate: &mut Gate, store: &Store, id: String) -> Vec<String> {
    let mut found = Vec::new();
    for filter in gate.stored_queries(&id) {
        match store.query(filter).await {
            Ok(events) => found.push(events),
            Err(error) => {
                eprintln!("read: subscription {id} could not be answered from the store: {error}");
                gate.forget(&id);
                return vec![
                    json!(["CLOSED", id, "error: the store could not be read"]).to_string(),
                ];
            }
        }
    }
    gate.stored_frames(&id, found)
}

/// Serve `client`, a connection already upgraded to WebSocket, until either
/// side closes it. `framework` is handed the far end of the pipe the framework
/// speaks on; `refusal` is what an `EVENT` is answered with.
pub(crate) async fn through<S, F, Fut>(
    client: S,
    refusal: Refusal,
    store: Store,
    framework: F,
) -> Result<(), RelayError>
where
    S: AsyncRead + AsyncWrite + Unpin,
    F: FnOnce(DuplexStream) -> Fut,
    Fut: Future<Output = Result<(), RelayError>> + Send + 'static,
{
    let config = WebSocketConfig::default()
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE));
    let mut client = WebSocketStream::from_raw_socket(client, Role::Server, Some(config)).await;
    let (near, far) = tokio::io::duplex(PIPE_BUFFER);
    let framework = tokio::spawn(framework(far));
    let mut inner = WebSocketStream::from_raw_socket(near, Role::Client, Some(config)).await;
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
                        if let Err(error) = client.send(Message::text(frame)).await {
                            break Err(error);
                        }
                    }
                    Verdict::Answer(frame) => {
                        if let Err(error) = client.send(Message::text(frame)).await {
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
                        Relayed::StoredDue(id) => stored_answer(&mut gate, &store, id).await,
                    };
                    let mut failed = None;
                    for frame in frames {
                        if let Err(error) = client.send(Message::text(frame)).await {
                            failed = Some(error);
                            break;
                        }
                    }
                    if let Some(error) = failed {
                        break Err(error);
                    }
                }
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break Ok(()),
                Some(Ok(_)) => {}
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
            Verdict::Refuse("error: invalid subscription id")
        );
    }

    #[test]
    fn what_is_not_a_request_is_left_to_the_framework() {
        let mut gate = Gate::default();
        for text in [
            "{not json",
            r#"{"a":1}"#,
            r#"["BOGUS","x"]"#,
            r#"["REQ",7,{}]"#,
        ] {
            assert_eq!(gate.client_sent(text), forwarded(text), "{text}");
        }
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
            Verdict::Refuse("error: too many subscriptions")
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
            Verdict::Refuse("error: too many subscriptions")
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
            Verdict::Refuse("error: too many filters")
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
    fn a_request_the_framework_can_answer_whole_is_not_watched() {
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
    fn a_filter_without_a_limit_is_asked_with_the_frameworks_default() {
        let mut gate = Gate {
            serves_expired: true,
            ..Gate::default()
        };
        gate.client_sent(r#"["REQ","a",{"kinds":[1]},{"limit":3},{"limit":9999}]"#);
        let limits: Vec<_> = gate
            .stored_queries("a")
            .iter()
            .map(|filter| filter.limit)
            .collect();
        assert_eq!(limits, vec![Some(500), Some(3), Some(500)]);
    }

    #[test]
    fn a_closed_subscription_is_no_longer_watched() {
        let mut gate = Gate {
            serves_expired: true,
            ..Gate::default()
        };
        gate.client_sent(r#"["REQ","a",{"kinds":[1]}]"#);
        gate.client_sent(r#"["CLOSE","a"]"#);
        assert!(gate.stored_queries("a").is_empty());
    }
}
