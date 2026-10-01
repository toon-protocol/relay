//! What the integration tests share: signed events, and a database the
//! TypeScript relay created.

// Each test binary uses a different part of this module.
#![allow(dead_code)]

use std::path::{Path, PathBuf};

use nostr::event::{Event, EventBuilder, FinalizeEvent, Kind, Tag};
use nostr::key::Keys;
use nostr::types::Timestamp;
use relay::VerifiedEvent;
use rusqlite::Connection;

/// The schema of a database the TypeScript relay created.
const TYPESCRIPT_SCHEMA: &str = include_str!("../fixtures/typescript-schema.sql");

/// An event of `kind` signed by `keys`, created at `created_at` (Unix seconds).
pub fn signed_by(keys: &Keys, kind: u16, created_at: u64, tags: &[&[&str]]) -> Event {
    let tags = tags
        .iter()
        .map(|tag| Tag::parse(tag.iter().copied()).expect("a non-empty tag parses"));
    EventBuilder::new(Kind::from(kind), "conformance")
        .tags(tags)
        .custom_created_at(Timestamp::from(created_at))
        .finalize(keys)
        .expect("a generated key signs an event")
}

/// An event of `kind` from a throwaway author.
pub fn signed(kind: u16, created_at: u64, tags: &[&[&str]]) -> Event {
    signed_by(&Keys::generate(), kind, created_at, tags)
}

pub fn verified(event: &Event) -> VerifiedEvent {
    VerifiedEvent::verify(event.clone()).expect("the event was just signed")
}

/// `events.db` in `dir`, created as the TypeScript relay creates it.
pub fn typescript_database(dir: &Path) -> PathBuf {
    let path = dir.join("events.db");
    let connection = Connection::open(&path).expect("a new file in a temp dir opens");
    connection
        .execute_batch(TYPESCRIPT_SCHEMA)
        .expect("the fixture is the schema SQLite recorded");
    path
}

/// Every statement SQLite has recorded for the database at `path`.
pub fn recorded_schema(path: &Path) -> Vec<String> {
    let connection = Connection::open(path).expect("the database opens");
    let mut statement = connection
        .prepare("SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY rowid")
        .expect("sqlite_master is always there");
    statement
        .query_map([], |row| row.get(0))
        .expect("the listing runs")
        .collect::<Result<_, _>>()
        .expect("every row has its statement")
}

/// A relay over a database the TypeScript relay created, with its read side
/// listening on a loopback port. The write side is driven through its router.
pub struct Running {
    pub relay: relay::Relay,
    /// `ws://127.0.0.1:<port>` of the read side.
    pub read_url: String,
    _data: tempfile::TempDir,
}

pub async fn running() -> Running {
    running_with(&[]).await
}

/// `running`, with `env` set as well.
pub async fn running_with(env: &[(&str, &str)]) -> Running {
    let data = tempfile::tempdir().expect("a temp dir");
    typescript_database(data.path());
    let data_dir = data.path().to_string_lossy().into_owned();
    let config = relay::Config::from_env(|name| match name {
        "TOON_SECRET_KEY" => Some("1".repeat(64)),
        "TOON_DATA_DIR" => Some(data_dir.clone()),
        _ => env
            .iter()
            .find(|(key, _)| *key == name)
            .map(|(_, value)| (*value).to_string()),
    })
    .expect("a secret key and a data directory are a complete configuration");
    let relay = relay::Relay::open(&config).expect("the TypeScript database opens");

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port is free");
    let address = listener.local_addr().expect("a bound listener has one");
    let router = relay.read_router();
    tokio::spawn(async move {
        let service = router.into_make_service_with_connect_info::<std::net::SocketAddr>();
        axum::serve(listener, service)
            .await
            .expect("the read side serves until the test ends");
    });
    Running {
        relay,
        read_url: format!("ws://{address}"),
        _data: data,
    }
}

/// `POST /write` with `body`, as the connector delivers a paid write.
pub async fn write(relay: &relay::Relay, body: impl Into<String>) -> (u16, serde_json::Value) {
    write_stating(relay, body, &[]).await
}

/// `POST /write` with `body` and `headers`: a delivery on which the connector
/// states the payment it verified.
pub async fn write_stating(
    relay: &relay::Relay,
    body: impl Into<String>,
    headers: &[(&str, &str)],
) -> (u16, serde_json::Value) {
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    let mut request =
        axum::http::Request::post("/write").header("content-type", "application/json");
    for (name, value) in headers {
        request = request.header(*name, *value);
    }
    let request = request
        .body(axum::body::Body::from(body.into()))
        .expect("a POST with a string body is a valid request");
    let response = relay
        .write_router()
        .oneshot(request)
        .await
        .expect("the router is infallible");
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("an in-memory body cannot fail to collect")
        .to_bytes();
    let body = serde_json::from_slice(&body).expect("every answer on the write port is JSON");
    (status, body)
}

/// The body the connector posts for `event`.
pub fn delivery(event: &Event) -> String {
    serde_json::json!({ "event": event }).to_string()
}

/// A NIP-01 client on the read side.
pub struct Client {
    socket: tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
}

impl Client {
    pub async fn connect(url: &str) -> Self {
        let (socket, _) = tokio_tungstenite::connect_async(url)
            .await
            .expect("the read side accepts the upgrade");
        Self { socket }
    }

    pub async fn send(&mut self, message: serde_json::Value) {
        use futures_util::SinkExt;
        self.socket
            .send(tokio_tungstenite::tungstenite::Message::text(
                message.to_string(),
            ))
            .await
            .expect("the connection is open");
    }

    /// The next frame from the relay, whatever it is, or `None` once the
    /// connection is closed or after two seconds of silence.
    pub async fn next_message(&mut self) -> Option<tokio_tungstenite::tungstenite::Message> {
        use futures_util::StreamExt;
        let wait = std::time::Duration::from_secs(2);
        tokio::time::timeout(wait, self.socket.next())
            .await
            .ok()??
            .ok()
    }

    /// The next frame from the relay, parsed, or `None` after two seconds
    /// of silence.
    pub async fn next(&mut self) -> Option<serde_json::Value> {
        use futures_util::StreamExt;
        let wait = std::time::Duration::from_secs(2);
        loop {
            let message = tokio::time::timeout(wait, self.socket.next())
                .await
                .ok()??;
            if let Ok(tokio_tungstenite::tungstenite::Message::Text(text)) = message {
                return Some(serde_json::from_str(&text).expect("a relay frame is JSON"));
            }
        }
    }

    /// Send a `REQ` and return the events that arrive before `EOSE`.
    pub async fn req(&mut self, id: &str, filter: serde_json::Value) -> Vec<serde_json::Value> {
        self.send(serde_json::json!(["REQ", id, filter])).await;
        let mut events = Vec::new();
        loop {
            let frame = self.next().await.expect("an EOSE arrives");
            match frame[0].as_str() {
                Some("EVENT") if frame[1] == id => events.push(frame[2].clone()),
                Some("EOSE") if frame[1] == id => return events,
                _ => {}
            }
        }
    }
}
