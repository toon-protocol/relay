//! The free read side: NIP-01 over WebSocket, on streams the relay accepted.
//!
//! The relay accepts the WebSocket upgrade on its own HTTP server (`read`) and
//! hands the upgraded stream to [`ReadSide::serve`], which is the whole of a
//! connection: one WebSocket endpoint, one task, and a [`Session`] that says
//! what each message is answered with. This module does what the session
//! cannot: it reads and writes the socket, asks the store, and listens to the
//! feed of live events the write side publishes to.
//!
//! One endpoint is the point (#237). Each connection used to cost three: the
//! relay's own facing the client, and one at each end of an in-memory pipe to
//! `nostr-sdk`'s `local_relay`, which the relay was built on and which gives
//! no way to size the 128 KiB read buffer of its end. With the relay's two at
//! 4 KiB (#231) an idle connection still held about 156 KiB, 128 KiB of it
//! that buffer: 322 MiB with 2000 idle subscribers, against 179 MiB for the
//! TypeScript image. Now there is no pipe and no second endpoint, a live
//! event goes from the feed to the client's socket in one step (#232), and
//! 2000 idle subscribers hold about 37 MiB (`soak/bench.mjs`, median of 3).
//!
//! Nothing a client writes reaches the store from here: the store is asked
//! with [`Store::query`] and nothing else, and an `EVENT` is refused in words
//! that name the Write Edge, because writes are paid and arrive on the write
//! port. A connection past the cap is closed with 1013.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Semaphore, broadcast};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode;
use tokio_tungstenite::tungstenite::protocol::{CloseFrame, Role, WebSocketConfig};
use tokio_tungstenite::tungstenite::{Error as SocketError, Message};

use crate::connector::EdgeSlot;
use crate::session::{LiveEvent, LiveFeed, NOTICE_BINARY, Refusal, Reply, Request, Session};
use crate::{Carriage, RelayError, Store, VerifiedEvent};

/// The largest message a client may send (5 MiB), and the largest frame.
const MAX_MESSAGE: usize = 5 * 1024 * 1024;
/// The read buffer of a connection's WebSocket endpoint.
///
/// tungstenite allocates this buffer when an endpoint is created and zero-fills
/// it before every read, one that finds nothing included, so at its default of
/// 128 KiB every page is resident for the life of the connection and filling
/// it is half the relay's CPU time during a fan-out (measured in #231 and
/// #232). A message larger than the buffer still arrives whole: tungstenite
/// grows the buffer to the frame it is reading.
const READ_BUFFER: usize = 4 * 1024;
/// The reason a connection past the cap is closed with.
const CLOSE_REASON_FULL: &str = "max connections reached";
/// The reason a connection that sends too many frames is closed with.
const CLOSE_REASON_FLOOD: &str = "rate-limited: too many messages";

/// The read side of one relay. Cheap to clone; every clone is the same one.
#[derive(Debug, Clone)]
pub(crate) struct ReadSide {
    store: Store,
    /// What an `EVENT` is refused with, on every connection.
    refusal: Refusal,
    connections: Arc<Semaphore>,
    /// What every connection delivers live events from.
    live: LiveFeed,
}

impl ReadSide {
    /// A read side that answers `REQ` from `store`, refuses `EVENT` towards
    /// the Write Edge in `edge` as it stands at the time, and holds at most
    /// `max_connections` connections at once.
    pub(crate) fn new(
        store: Store,
        edge: EdgeSlot,
        write_carriage: Option<Carriage>,
        max_connections: usize,
    ) -> Self {
        Self {
            store,
            refusal: Refusal {
                edge,
                write_carriage,
            },
            // More permits than a semaphore can hold is no cap at all.
            connections: Arc::new(Semaphore::new(max_connections.min(Semaphore::MAX_PERMITS))),
            live: LiveFeed::new(),
        }
    }

    /// Speak NIP-01 with `peer` on `stream`, a connection already upgraded to
    /// WebSocket, until either side closes it. A connection past the cap is
    /// closed with 1013 instead.
    pub(crate) async fn serve<S>(&self, stream: S, peer: SocketAddr) -> Result<(), RelayError>
    where
        S: AsyncRead + AsyncWrite + Unpin,
    {
        let Ok(_held) = Arc::clone(&self.connections).try_acquire_owned() else {
            eprintln!("read: connection from {peer} refused: the connection cap is reached");
            return refuse_full(stream).await;
        };
        // Before anything is asked: no event published from here on is missed.
        let mut live = self.live.listen();
        let mut client =
            WebSocketStream::from_raw_socket(stream, Role::Server, Some(socket_config())).await;
        let mut session = Session::new(self.refusal.clone());

        let ended = loop {
            tokio::select! {
                // Live events first: one published before a request was read
                // is delivered before the request is answered, so it is not
                // sent again, live, to the subscription that request opens.
                biased;
                event = live.recv() => {
                    let frames = match event {
                        Ok(event) => session.live_frames(&event).collect(),
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            live = live.resubscribe();
                            session.overflowed()
                        }
                        // The relay is going: every clone of the feed is dropped.
                        Err(broadcast::error::RecvError::Closed) => break Ok(()),
                    };
                    if let Err(error) = send_all(&mut client, frames).await {
                        break Err(error);
                    }
                }
                // A request is answered whole in this arm, so an event that
                // arrives during a stored answer waits in the feed and is
                // delivered after its EOSE, never inside it.
                frame = client.next() => {
                    let message = match frame {
                        Some(Ok(message)) => message,
                        None => break Ok(()),
                        Some(Err(error)) => break Err(error),
                    };
                    if !session.frame_arrived(Instant::now()) {
                        eprintln!("read: connection from {peer} closed: it sent too many messages");
                        let _ = client
                            .close(Some(CloseFrame {
                                code: CloseCode::Policy,
                                reason: CLOSE_REASON_FLOOD.into(),
                            }))
                            .await;
                        return Ok(());
                    }
                    let frames = match message {
                        Message::Text(text) => match session.client_sent(&text, Instant::now()) {
                            Reply::Frames(frames) => frames,
                            Reply::Ask(request) => {
                                self.answer(&mut session, &mut live, request).await
                            }
                        },
                        Message::Binary(_) => vec![json!(["NOTICE", NOTICE_BINARY]).to_string()],
                        Message::Close(_) => break Ok(()),
                        // Pings are answered by the WebSocket layer itself.
                        _ => Vec::new(),
                    };
                    if let Err(error) = send_all(&mut client, frames).await {
                        break Err(error);
                    }
                }
            }
        };

        // Say goodbye to a client that is still there.
        let _ = client.close(None).await;
        match ended {
            Ok(()) | Err(SocketError::AlreadyClosed | SocketError::ConnectionClosed) => Ok(()),
            Err(error) => Err(RelayError::ReadSide(error.to_string())),
        }
    }

    /// The frames that answer `request`: what the store holds for it and
    /// `EOSE`, then what waited in the feed meanwhile for the open
    /// subscriptions; or the `CLOSED` a client is told when the store cannot
    /// be read; or, when the connection fell behind the feed, the overflow
    /// `CLOSED` for every subscription.
    ///
    /// The queries run after every event that was saved has been published
    /// (the write side publishes inside the store's exclusive section), so
    /// what the feed holds once they are done covers every event they may
    /// have found. Those are taken, without waiting, and dealt with as the
    /// answer is built.
    async fn answer(
        &self,
        session: &mut Session,
        live: &mut broadcast::Receiver<Arc<LiveEvent>>,
        request: Request,
    ) -> Vec<String> {
        let mut found = Vec::new();
        for filter in request.queries() {
            match self.store.query(filter).await {
                Ok(events) => found.push(events),
                Err(error) => {
                    eprintln!(
                        "read: subscription {} could not be answered from the store: {error}",
                        request.id()
                    );
                    return session.unanswered(request);
                }
            }
        }
        let mut waiting = Vec::new();
        loop {
            match live.try_recv() {
                Ok(event) => waiting.push(event),
                Err(broadcast::error::TryRecvError::Lagged(_)) => {
                    *live = live.resubscribe();
                    return session.overflowed_during(request);
                }
                // Empty, or the relay is going: the loop will see which.
                Err(_) => break,
            }
        }
        session.answered(request, found, &waiting)
    }

    /// Deliver `event` to every open subscription it matches. Nothing is
    /// saved: the caller has saved it, or it is not to be kept.
    pub(crate) fn deliver(&self, event: &VerifiedEvent) {
        self.live.publish(event.event());
    }
}

/// What a connection's endpoint is built from.
fn socket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .read_buffer_size(READ_BUFFER)
        .max_message_size(Some(MAX_MESSAGE))
        .max_frame_size(Some(MAX_MESSAGE))
}

/// Send each of `frames` to the client, in order, until one cannot be sent.
async fn send_all<S>(
    client: &mut WebSocketStream<S>,
    frames: impl IntoIterator<Item = String>,
) -> Result<(), SocketError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    for frame in frames {
        client.send(Message::text(frame)).await?;
    }
    Ok(())
}

/// Close a connection the relay has no room for: 1013, "try again later".
async fn refuse_full<S>(client: S) -> Result<(), RelayError>
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

#[cfg(test)]
mod tests {
    use nostr::event::{Event, EventBuilder, FinalizeEvent, Kind};
    use serde_json::Value;
    use tokio::io::DuplexStream;

    use super::*;
    use crate::session::MESSAGES_PER_MINUTE;

    #[test]
    fn a_connections_endpoint_does_not_use_the_default_read_buffer() {
        let config = socket_config();
        assert_eq!(config.read_buffer_size, READ_BUFFER);
        assert!(config.read_buffer_size < WebSocketConfig::default().read_buffer_size);
        assert_eq!(config.max_message_size, Some(MAX_MESSAGE));
        assert_eq!(config.max_frame_size, Some(MAX_MESSAGE));
    }

    /// A read side over a new, empty store, and one client connected to it
    /// over an in-memory stream.
    struct Connection {
        read_side: ReadSide,
        client: WebSocketStream<DuplexStream>,
        _data: tempfile::TempDir,
    }

    impl Connection {
        async fn open() -> Self {
            let data = tempfile::tempdir().expect("a temp dir");
            let store = Store::open(&data.path().join("events.db")).expect("a new database opens");
            let read_side = ReadSide::new(store, EdgeSlot::default(), None, 1);
            let (ours, theirs) = tokio::io::duplex(64 * 1024);
            let serving = read_side.clone();
            tokio::spawn(async move {
                let peer = SocketAddr::from(([127, 0, 0, 1], 0));
                serving.serve(theirs, peer).await
            });
            let client = WebSocketStream::from_raw_socket(ours, Role::Client, None).await;
            Self {
                read_side,
                client,
                _data: data,
            }
        }

        async fn send(&mut self, message: Message) {
            self.client
                .send(message)
                .await
                .expect("the relay is listening");
        }

        /// The next message the client is sent, or `None` after a second of
        /// silence.
        async fn next_message(&mut self) -> Option<Message> {
            let wait = std::time::Duration::from_secs(1);
            tokio::time::timeout(wait, self.client.next())
                .await
                .ok()??
                .ok()
        }

        /// The next text frame, parsed.
        async fn next(&mut self) -> Option<Value> {
            match self.next_message().await? {
                Message::Text(text) => {
                    Some(serde_json::from_str(&text).expect("a relay frame is JSON"))
                }
                other => panic!("expected a text frame, got {other:?}"),
            }
        }

        /// Publish `event` as the write side does, without saving it.
        fn publish(&self, event: &Event) {
            self.read_side.live.publish(event);
        }
    }

    fn event(content: &str) -> Event {
        EventBuilder::new(Kind::from(1), content)
            .finalize(&nostr::key::Keys::generate())
            .expect("a generated key signs an event")
    }

    #[tokio::test]
    async fn a_message_larger_than_the_read_buffer_passes_both_ways() {
        let mut connection = Connection::open().await;
        // In: a request far larger than the buffer is read whole and answered.
        let request = json!(["REQ", "a", { "#t": ["x".repeat(64 * READ_BUFFER)] }, {}]);
        connection.send(Message::text(request.to_string())).await;
        assert_eq!(connection.next().await, Some(json!(["EOSE", "a"])));
        // Out: so is an event far larger than it.
        let large = event(&"y".repeat(64 * READ_BUFFER));
        connection.publish(&large);
        assert_eq!(connection.next().await, Some(json!(["EVENT", "a", large])));
    }

    #[tokio::test]
    async fn an_event_published_before_a_request_is_read_is_not_heard_by_it() {
        let mut connection = Connection::open().await;
        // A round trip: the connection is being served, and listens.
        connection
            .send(Message::text(r#"["REQ","other",{"kinds":[7]}]"#))
            .await;
        assert_eq!(connection.next().await, Some(json!(["EOSE", "other"])));

        connection.publish(&event("too early"));
        connection.send(Message::text(r#"["REQ","a",{}]"#)).await;
        assert_eq!(connection.next().await, Some(json!(["EOSE", "a"])));
        let live = event("live");
        connection.publish(&live);
        assert_eq!(connection.next().await, Some(json!(["EVENT", "a", live])));
        assert_eq!(connection.next_message().await, None);
    }

    /// A read side over an empty store, a session on it with `open` already
    /// subscribed, and a listener on its feed: a connection's parts, driven
    /// without the socket so the order of things is forced.
    struct Direct {
        read_side: ReadSide,
        session: Session,
        feed: broadcast::Receiver<Arc<LiveEvent>>,
        _data: tempfile::TempDir,
    }

    impl Direct {
        async fn open(open: &[&str]) -> Self {
            let data = tempfile::tempdir().expect("a temp dir");
            let store = Store::open(&data.path().join("events.db")).expect("a new database opens");
            let read_side = ReadSide::new(store, EdgeSlot::default(), None, 1);
            let feed = read_side.live.listen();
            let mut direct = Self {
                read_side,
                session: Session::new(Refusal::default()),
                feed,
                _data: data,
            };
            for req in open {
                direct.req(req).await;
            }
            direct
        }

        async fn req(&mut self, text: &str) -> Vec<Value> {
            let Reply::Ask(request) = self.session.client_sent(text, Instant::now()) else {
                panic!("{text} asks the store");
            };
            let frames = self
                .read_side
                .answer(&mut self.session, &mut self.feed, request)
                .await;
            frames
                .iter()
                .map(|frame| serde_json::from_str(frame).expect("a frame is JSON"))
                .collect()
        }

        /// Save `event` and publish it inside the store's exclusive section,
        /// as the write side does.
        async fn save(&self, event: &Event) {
            let verified = VerifiedEvent::verify(event.clone()).expect("signed");
            let read_side = self.read_side.clone();
            self.read_side
                .store
                .save_then(&verified, move |event| read_side.deliver(event))
                .await
                .expect("saved");
        }

        /// What the feed delivers to the session now, as frames.
        fn live(&mut self) -> Vec<Value> {
            let mut frames = Vec::new();
            while let Ok(event) = self.feed.try_recv() {
                frames.extend(self.session.live_frames(&event));
            }
            frames
                .iter()
                .map(|frame| serde_json::from_str(frame).expect("a frame is JSON"))
                .collect()
        }
    }

    #[tokio::test]
    async fn an_event_saved_and_published_before_a_request_is_sent_once() {
        let mut direct = Direct::open(&[]).await;
        let saved = event("saved");
        direct.save(&saved).await;
        let frames = direct.req(r#"["REQ","a",{}]"#).await;
        assert_eq!(
            frames,
            vec![json!(["EVENT", "a", saved]), json!(["EOSE", "a"])]
        );
        assert_eq!(direct.live(), Vec::<Value>::new());
    }

    #[tokio::test]
    async fn an_event_published_while_a_request_is_answered_goes_live_after_eose() {
        let mut direct = Direct::open(&[r#"["REQ","other",{}]"#]).await;
        // In the feed but not in the store: published after the queries.
        let late = event("late");
        direct
            .read_side
            .deliver(&VerifiedEvent::verify(late.clone()).expect("signed"));
        let frames = direct.req(r#"["REQ","a",{}]"#).await;
        assert_eq!(
            frames,
            vec![
                json!(["EOSE", "a"]),
                json!(["EVENT", "other", late]),
                json!(["EVENT", "a", late]),
            ]
        );
        assert_eq!(direct.live(), Vec::<Value>::new());
    }

    #[tokio::test]
    async fn a_request_that_finds_the_feed_overrun_closes_every_subscription() {
        let mut direct = Direct::open(&[r#"["REQ","other",{"kinds":[7]}]"#]).await;
        let one = VerifiedEvent::verify(event("one of too many")).expect("signed");
        for _ in 0..=2048 {
            direct.read_side.deliver(&one);
        }
        let frames = direct.req(r#"["REQ","a",{"kinds":[7]}]"#).await;
        let mut closed: Vec<_> = frames
            .iter()
            .map(|f| (f[0].clone(), f[1].clone()))
            .collect();
        closed.sort_by_key(|(_, id)| id.to_string());
        assert_eq!(
            closed,
            vec![
                (json!("CLOSED"), json!("a")),
                (json!("CLOSED"), json!("other"))
            ]
        );
        assert_eq!(direct.live(), Vec::<Value>::new());
    }

    #[tokio::test]
    async fn a_binary_frame_is_a_notice_and_the_connection_stays() {
        let mut connection = Connection::open().await;
        connection.send(Message::binary(vec![1, 2, 3])).await;
        assert_eq!(
            connection.next().await,
            Some(json!(["NOTICE", NOTICE_BINARY]))
        );
        connection.send(Message::text(r#"["REQ","a",{}]"#)).await;
        assert_eq!(connection.next().await, Some(json!(["EOSE", "a"])));
    }

    #[tokio::test]
    async fn a_connection_a_whole_feed_behind_is_told_its_subscriptions_are_closed() {
        let mut connection = Connection::open().await;
        connection
            .send(Message::text(r#"["REQ","a",{"kinds":[7]}]"#))
            .await;
        assert_eq!(connection.next().await, Some(json!(["EOSE", "a"])));

        // More than the feed holds, published before the connection's task
        // runs again: none of them is one the subscription asked for, so
        // nothing but the missed ones can close it.
        let event = event("one of too many");
        for _ in 0..=2048 {
            connection.publish(&event);
        }
        let closed = connection.next().await.expect("the client is told");
        assert_eq!(closed[0], "CLOSED");
        assert_eq!(closed[1], "a");
        assert_eq!(connection.next_message().await, None);
    }

    #[tokio::test]
    async fn a_connection_that_sends_too_many_frames_is_closed_as_a_policy_violation() {
        let mut connection = Connection::open().await;
        // Twice a minute's worth of the smallest frame there is: the
        // allowance refills while they are sent, by far less than that.
        for _ in 0..2 * MESSAGES_PER_MINUTE {
            let ping = Message::Ping(Vec::new().into());
            if connection.client.feed(ping).await.is_err() {
                break;
            }
        }
        let _ = connection.client.flush().await;
        loop {
            match connection.next_message().await {
                Some(Message::Pong(_)) => {}
                Some(Message::Close(Some(frame))) => {
                    assert_eq!(frame.code, CloseCode::Policy);
                    assert_eq!(frame.reason.as_str(), CLOSE_REASON_FLOOD);
                    break;
                }
                other => panic!("expected to be closed for flooding, got {other:?}"),
            }
        }
    }
}
