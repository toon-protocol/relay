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
//! - a connection past the cap is closed with 1013.
//!
//! This module imports nothing of the framework: it sees a stream to hand
//! over, which is what keeps the framework behind its one adapter.

use std::collections::HashSet;
use std::future::Future;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};

use crate::RelayError;

/// The most subscriptions one connection holds. Replacing one is not another.
pub(crate) const MAX_SUBSCRIPTIONS: usize = 20;
/// The most filters one REQ carries.
pub(crate) const MAX_FILTERS: usize = 10;

/// The framework's own message ceiling (5 MiB), so the gate is not the
/// smaller limit.
const MAX_MESSAGE: usize = 5 * 1024 * 1024;
/// How much the pipe to the framework buffers each way.
const PIPE_BUFFER: usize = 64 * 1024;
/// The reason a connection past the cap is closed with.
const FULL: &str = "max connections reached";

/// What the gate does with one message from a client.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Verdict {
    /// Pass this text to the framework.
    Forward(String),
    /// Answer with this `NOTICE`; the framework never hears the message.
    Refuse(&'static str),
}

/// One connection's view of its own subscriptions.
#[derive(Debug, Default)]
pub(crate) struct Gate {
    open: HashSet<String>,
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
                self.open.insert(id);
                let mut changed = false;
                for filter in items.iter_mut().skip(2) {
                    changed |= keep_whole_values(filter);
                }
                Verdict::Forward(if changed {
                    Value::Array(items).to_string()
                } else {
                    text.to_string()
                })
            }
            _ => Verdict::Forward(text.to_string()),
        }
    }

    /// Note `text`, a message the framework sent: a subscription it closed is
    /// no longer open, whoever closed it.
    pub(crate) fn relay_sent(&mut self, text: &str) {
        if !text.starts_with("[\"CLOSED\"") {
            return;
        }
        if let Ok(Value::Array(items)) = serde_json::from_str::<Value>(text)
            && let Some(id) = items.get(1).and_then(Value::as_str)
        {
            self.open.remove(id);
        }
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
            reason: FULL.into(),
        }))
        .await;
    Ok(())
}

/// Serve `client`, a connection already upgraded to WebSocket, until either
/// side closes it. `framework` is handed the far end of the pipe the framework
/// speaks on.
pub(crate) async fn through<S, F, Fut>(client: S, framework: F) -> Result<(), RelayError>
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
    let mut gate = Gate::default();

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
                    gate.relay_sent(&text);
                    if let Err(error) = client.send(Message::Text(text)).await {
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
}
