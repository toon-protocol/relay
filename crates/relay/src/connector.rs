//! Reading the Write Edge from the connector, in the background (#199).
//!
//! The relay asks its connector for the connector's own facts, free, on
//! `GET /ilp` (connector ADR 0050), and keeps what [`WriteEdge::read`] makes
//! of the answer. Nothing here waits for the connector and nothing here can
//! fail the relay: the canonical compose bundle starts the connector only
//! once the relay is healthy, so a relay that waited for it would deadlock its
//! own deployment, and a connector that goes away later must not take free
//! reads with it. While the edge is unknown the information document carries
//! no `toon` object, which is the truth.
//!
//! The poll is quick while the edge is unknown (the ordinary reason is the
//! first seconds of a boot) and slow once it is known. It logs on change
//! only, so a connector that stays down is one line, not one every five
//! seconds.
//!
//! ## Reading the document
//!
//! The document is read in the connector's own types, but not all-or-nothing.
//! A strictly typed parse fails the whole document on one settlement entry
//! the pinned types do not know (a chain a newer connector image adds), and
//! the relay would advertise nothing until the pin moved. So the lists the
//! relay only copies from are read entry by entry and an entry that does not
//! read is left out, as the TypeScript reader leaves it out: a route the
//! relay cannot read is one it cannot confirm, and a settlement it cannot
//! read is one it does not list. What the document must have to be a
//! self-description at all (its versions) stays required.

use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use connector_domain::node::NodeSelfDescription;
use connector_domain::x402::X402BatchSettlementTerms;
use http_body_util::{BodyExt, Empty, Limited};
use hyper::body::Bytes;
use hyper::header::ACCEPT;
use hyper::{Request, StatusCode, Uri};
use hyper_util::client::legacy::Client;
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::rt::TokioExecutor;
use serde_json::Value;

use crate::config::EdgeSettings;
use crate::{RelayError, WriteEdge};

/// How often an unknown edge is retried.
pub(crate) const RETRY: Duration = Duration::from_secs(5);

/// How often a known edge is read again.
pub(crate) const REFRESH: Duration = Duration::from_secs(300);

/// The longest a single read may take, connecting included.
const TIMEOUT: Duration = Duration::from_secs(10);

/// A self-description is a few kilobytes; this is for a connector that is not
/// one.
const MAX_BODY: usize = 1 << 20;

/// The edge as last read, shared by the poll that writes it and the
/// document that renders it. `None` while it is unknown.
#[derive(Debug, Clone, Default)]
pub(crate) struct EdgeSlot(Arc<RwLock<Option<Arc<WriteEdge>>>>);

impl EdgeSlot {
    pub(crate) fn current(&self) -> Option<Arc<WriteEdge>> {
        self.0
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set(&self, edge: Option<WriteEdge>) {
        *self.0.write().unwrap_or_else(PoisonError::into_inner) = edge.map(Arc::new);
    }
}

/// How often to ask.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Intervals {
    pub(crate) unknown: Duration,
    pub(crate) known: Duration,
}

impl Default for Intervals {
    fn default() -> Self {
        Self {
            unknown: RETRY,
            known: REFRESH,
        }
    }
}

/// Ask the connector for the edge until the task is aborted. Never returns
/// and never fails: every outcome is a state of `slot`.
pub(crate) async fn watch(connector: EdgeSettings, intervals: Intervals, slot: EdgeSlot) {
    let client = Client::builder(TokioExecutor::new()).build_http();
    let mut reported: Option<String> = None;
    loop {
        let reading = read(&client, &connector).await;
        let report = match &reading {
            Ok(edge) => format!(
                "[relay] paid write edge: {} at {}{}, {} uusdc per write, sealed to {}…",
                edge.ilp_address(),
                edge.connector_url(),
                edge.carriage()
                    .map_or(" (no carriage pinned)".to_string(), |carriage| format!(
                        " over {}",
                        carriage.as_str()
                    )),
                edge.price(),
                edge.seal_key().chars().take(18).collect::<String>(),
            ),
            Err(error) => format!(
                "[relay] paid write edge UNKNOWN: {error}. Until this is fixed the NIP-11 \
                 document names no edge."
            ),
        };
        if reported.as_ref() != Some(&report) {
            println!("{report}");
            reported = Some(report);
        }
        let wait = if reading.is_ok() {
            intervals.known
        } else {
            intervals.unknown
        };
        slot.set(reading.ok());
        tokio::time::sleep(wait).await;
    }
}

type HttpClient = Client<HttpConnector, Empty<Bytes>>;

async fn read(client: &HttpClient, connector: &EdgeSettings) -> Result<WriteEdge, RelayError> {
    let unreadable = |reason: String| RelayError::ConnectorUnreadable {
        url: connector.connector_url.clone(),
        reason,
    };
    let body = tokio::time::timeout(TIMEOUT, fetch(client, &connector.connector_url))
        .await
        .map_err(|_| unreadable("it did not answer in time".to_string()))?
        .map_err(unreadable)?;
    let description = parse(&body).map_err(|reason| {
        unreadable(format!("it is not a connector self-description: {reason}"))
    })?;
    WriteEdge::read(&connector.write_ilp_address, &description)
}

async fn fetch(client: &HttpClient, url: &str) -> Result<Bytes, String> {
    let uri: Uri = url.parse().map_err(|error| format!("{error}"))?;
    let request = Request::get(uri)
        .header(ACCEPT, "application/json")
        .body(Empty::new())
        .map_err(|error| error.to_string())?;
    let response = client
        .request(request)
        .await
        .map_err(|error| error.to_string())?;
    if response.status() != StatusCode::OK {
        return Err(format!("it answered HTTP {}", response.status().as_u16()));
    }
    let body = Limited::new(response.into_body(), MAX_BODY)
        .collect()
        .await
        .map_err(|error| error.to_string())?;
    Ok(body.to_bytes())
}

/// The connector's self-description out of the body of its `GET /ilp`, with
/// the entries it lists that these types cannot read left out.
pub(crate) fn parse(body: &[u8]) -> Result<NodeSelfDescription, String> {
    let mut document: Value = serde_json::from_slice(body).map_err(|error| error.to_string())?;
    let fields = document
        .as_object_mut()
        .ok_or("the body is not a JSON object")?;
    let routes = readable(fields.remove("routes"));
    let settlements = readable::<X402BatchSettlementTerms>(fields.remove("batchSettlements"));
    // Not the relay's to read: nothing renders them.
    fields.remove("voucherSigners");
    let mut description: NodeSelfDescription =
        serde_json::from_value(document).map_err(|error| error.to_string())?;
    description.routes = routes;
    description.batch_settlements = settlements;
    Ok(description)
}

/// The entries of a JSON array that read as `T`, in order. Anything else is
/// no entries.
fn readable<T: serde::de::DeserializeOwned>(list: Option<Value>) -> Vec<T> {
    match list {
        Some(Value::Array(entries)) => entries
            .into_iter()
            .filter_map(|entry| serde_json::from_value(entry).ok())
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use std::net::SocketAddr;
    use std::sync::Mutex;

    use axum::Router;
    use axum::http::StatusCode;
    use axum::routing::get;
    use serde_json::json;
    use tokio::net::TcpListener;

    use super::*;
    use crate::Carriage;
    use crate::document::{Document, Settings, write_refusal};

    const SEAL_KEY: &str = "0x04abababababababababababababababababababababababababababababababab";

    fn evm() -> Value {
        json!({
            "network": "eip155:84532",
            "asset": "0x036cbd53842c5426634e7929541ec2318f3dcf7e",
            "payTo": "0x1111111111111111111111111111111111111111",
            "receiverAuthorizer": "0x1111111111111111111111111111111111111111",
            "withdrawDelay": 86400,
            "name": "USDC",
            "version": "2",
            "assetTransferMethod": "eip3009",
            "facilitator": "https://facilitator.example"
        })
    }

    fn document() -> Value {
        json!({
            "httpEndpoint": "https://relay.example/ilp",
            "edgeIdentity": { "keyId": "edge-1", "publicKey": SEAL_KEY },
            "batchSettlements": [evm()],
            "routes": [
                { "prefix": "g.toon.relay", "price": "1000", "requiredTransport": "http" },
                { "prefix": "g.toon.relay.store", "price": "2000" }
            ],
            "supportedVersions": [1],
            "defaultVersion": 1
        })
    }

    fn settings() -> Settings {
        Settings {
            pubkey: "ab".repeat(32),
            name: None,
            description: None,
            contact: None,
            write_carriage: None,
            enforce_expiration: true,
        }
    }

    /// The connector's `GET /ilp`, serving whatever `answer` holds.
    async fn stub(answer: Arc<Mutex<(StatusCode, String)>>) -> SocketAddr {
        let router = Router::new().route(
            "/ilp",
            get(move || {
                let answer = answer
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                async move { answer }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a free port on loopback");
        let address = listener
            .local_addr()
            .expect("a bound listener has an address");
        tokio::spawn(async move { axum::serve(listener, router).await });
        address
    }

    fn quickly() -> Intervals {
        Intervals {
            unknown: Duration::from_millis(20),
            known: Duration::from_millis(20),
        }
    }

    async fn eventually(slot: &EdgeSlot, known: bool) -> Option<Arc<WriteEdge>> {
        for _ in 0..200 {
            let edge = slot.current();
            if edge.is_some() == known {
                return edge;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        panic!(
            "the edge never became {}",
            if known { "known" } else { "unknown" }
        );
    }

    #[test]
    fn an_entry_these_types_cannot_read_is_left_out_and_the_rest_is_read() {
        let mut body = document();
        body["batchSettlements"] = json!([
            { "network": "newchain:1", "asset": "x", "somethingNew": true },
            evm()
        ]);
        body["routes"] = json!([{ "prefix": "g.toon.relay" }, { "prefix": "g.a", "price": "1" }]);
        body["voucherSigners"] = json!([{ "unknown": "shape" }]);
        let parsed = parse(body.to_string().as_bytes()).expect("the rest of the document reads");
        assert_eq!(parsed.batch_settlements.len(), 1);
        assert_eq!(parsed.routes.len(), 1);
        assert_eq!(parsed.routes[0].prefix, "g.a");
    }

    #[test]
    fn a_body_that_is_not_a_self_description_is_refused() {
        assert!(parse(b"not json").is_err());
        assert!(parse(b"[]").is_err());
        let mut body = document();
        body.as_object_mut()
            .expect("an object")
            .remove("supportedVersions");
        assert!(parse(body.to_string().as_bytes()).is_err());
    }

    #[tokio::test]
    async fn the_edge_is_learned_from_the_connector_and_rendered_from_it() {
        let answer = Arc::new(Mutex::new((StatusCode::OK, document().to_string())));
        let address = stub(answer).await;
        let slot = EdgeSlot::default();
        let task = tokio::spawn(watch(
            EdgeSettings {
                connector_url: format!("http://{address}/ilp"),
                write_ilp_address: "g.toon.relay".to_string(),
            },
            quickly(),
            slot.clone(),
        ));

        let edge = eventually(&slot, true).await.expect("the edge is known");
        assert_eq!(edge.price(), 1000);
        let rendered = serde_json::to_value(Document::render(&settings(), Some(&edge)))
            .expect("the document is JSON");
        assert_eq!(
            rendered["toon"],
            json!({
                "ilp_address": "g.toon.relay",
                "connector_url": "https://relay.example/ilp",
                "connector_seal_key": SEAL_KEY,
                "carriage": "http",
                "price": 1000,
                "settlement": [{
                    "network": "eip155:84532",
                    "asset": "0x036cbd53842c5426634e7929541ec2318f3dcf7e"
                }]
            })
        );
        assert_eq!(
            rendered["fees"],
            json!({ "publication": [{ "amount": 1000, "unit": "uusdc" }] })
        );
        assert_eq!(rendered["limitation"]["payment_required"], json!(true));
        task.abort();
    }

    #[tokio::test]
    async fn a_connector_that_comes_up_late_and_goes_away_is_followed_and_never_fatal() {
        let answer = Arc::new(Mutex::new((StatusCode::SERVICE_UNAVAILABLE, String::new())));
        let address = stub(answer.clone()).await;
        let slot = EdgeSlot::default();
        let task = tokio::spawn(watch(
            EdgeSettings {
                connector_url: format!("http://{address}/ilp"),
                write_ilp_address: "g.toon.relay".to_string(),
            },
            quickly(),
            slot.clone(),
        ));
        assert!(eventually(&slot, false).await.is_none());

        *answer.lock().unwrap_or_else(PoisonError::into_inner) =
            (StatusCode::OK, document().to_string());
        eventually(&slot, true).await;

        *answer.lock().unwrap_or_else(PoisonError::into_inner) =
            (StatusCode::OK, "garbage".to_string());
        eventually(&slot, false).await;
        task.abort();
    }

    #[tokio::test]
    async fn an_address_the_connector_does_not_terminate_is_never_advertised() {
        let answer = Arc::new(Mutex::new((StatusCode::OK, document().to_string())));
        let address = stub(answer).await;
        let slot = EdgeSlot::default();
        let task = tokio::spawn(watch(
            EdgeSettings {
                connector_url: format!("http://{address}/ilp"),
                write_ilp_address: "g.toon.elsewhere".to_string(),
            },
            quickly(),
            slot.clone(),
        ));
        // The same connector, asked for the address it does terminate, is
        // read: so the other is refused for its route, not for no answer.
        let terminated = EdgeSlot::default();
        let control = tokio::spawn(watch(
            EdgeSettings {
                connector_url: format!("http://{address}/ilp"),
                write_ilp_address: "g.toon.relay".to_string(),
            },
            quickly(),
            terminated.clone(),
        ));
        eventually(&terminated, true).await;
        tokio::time::sleep(Duration::from_millis(100)).await;
        assert!(slot.current().is_none());
        task.abort();
        control.abort();
    }

    #[tokio::test]
    async fn a_connector_nobody_listens_for_is_an_unknown_edge() {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
        let address = listener.local_addr().expect("an address");
        drop(listener);
        let client = Client::builder(TokioExecutor::new()).build_http();
        let error = read(
            &client,
            &EdgeSettings {
                connector_url: format!("http://{address}/ilp"),
                write_ilp_address: "g.toon.relay".to_string(),
            },
        )
        .await
        .expect_err("nothing is listening");
        assert!(matches!(error, RelayError::ConnectorUnreadable { .. }));
    }

    #[test]
    fn the_operators_carriage_fills_the_connectors_silence_and_never_overrides_it() {
        let mut body = document();
        body["routes"] = json!([{ "prefix": "g.toon.relay", "price": "0" }]);
        let parsed = parse(body.to_string().as_bytes()).expect("a document");
        let edge = WriteEdge::read("g.toon.relay", &parsed).expect("an edge");
        let mut operator = settings();
        operator.write_carriage = Some(Carriage::Btp);
        let rendered =
            serde_json::to_value(Document::render(&operator, Some(&edge))).expect("JSON");
        assert_eq!(rendered["toon"]["carriage"], json!("btp"));
        assert_eq!(rendered["toon"]["price"], json!(0));
        assert!(rendered.get("fees").is_none());
        assert_eq!(rendered["limitation"]["payment_required"], json!(false));

        let silent =
            serde_json::to_value(Document::render(&settings(), Some(&edge))).expect("JSON");
        assert!(silent["toon"].get("carriage").is_none());
    }

    #[test]
    fn a_websocket_write_is_refused_towards_the_edge_the_document_names() {
        let parsed = parse(document().to_string().as_bytes()).expect("a document");
        let edge = WriteEdge::read("g.toon.relay", &parsed).expect("an edge");
        let refusal = write_refusal(Some(&edge), None);
        assert!(
            refusal.starts_with("writes require ILP payment"),
            "{refusal}"
        );
        for named in [
            "g.toon.relay",
            "https://relay.example/ilp",
            " over http",
            "1000 uusdc",
            "application/nostr+json",
        ] {
            assert!(refusal.contains(named), "{refusal} names {named}");
        }

        let mut body = document();
        body["routes"] = json!([{ "prefix": "g.toon.relay", "price": "0" }]);
        let parsed = parse(body.to_string().as_bytes()).expect("a document");
        let free = WriteEdge::read("g.toon.relay", &parsed).expect("an edge");
        let refusal = write_refusal(Some(&free), Some(Carriage::Btp));
        assert!(!refusal.contains("require ILP payment"), "{refusal}");
        assert!(
            refusal.contains("free") && refusal.contains(" over btp"),
            "{refusal}"
        );

        let unknown = write_refusal(None, Some(Carriage::Btp));
        assert!(unknown.starts_with("writes require ILP payment, and this relay does not"));
    }

    #[test]
    fn without_an_edge_there_is_no_toon_object() {
        let rendered = serde_json::to_value(Document::render(&settings(), None)).expect("JSON");
        assert!(rendered.get("toon").is_none());
        assert!(rendered.get("fees").is_none());
        assert_eq!(rendered["supported_nips"], json!([1, 9, 11, 16, 40]));
        let mut off = settings();
        off.enforce_expiration = false;
        let rendered = serde_json::to_value(Document::render(&off, None)).expect("JSON");
        assert_eq!(rendered["supported_nips"], json!([1, 9, 11, 16]));
    }
}
