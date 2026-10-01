//! The read port's HTTP face. The relay owns this server: it accepts the
//! WebSocket upgrade itself and hands the upgraded stream to the read side,
//! so the framework never binds a listener (#185).
//!
//! Any request that is not a WebSocket upgrade is answered `426 Upgrade
//! Required`, as the TypeScript relay answers it and as fleet health checks
//! expect (story 28). The Relay Information Document, which shares this port,
//! arrives with the connector edge (#199).

use std::net::{Ipv4Addr, SocketAddr};

use axum::extract::{ConnectInfo, Request, State};
use axum::http::header::{
    CONNECTION, CONTENT_TYPE, SEC_WEBSOCKET_ACCEPT, SEC_WEBSOCKET_KEY, SEC_WEBSOCKET_VERSION,
    UPGRADE,
};
use axum::http::{HeaderMap, HeaderName, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hyper::upgrade::OnUpgrade;
use hyper_util::rt::TokioIo;
use sha1::{Digest, Sha1};

use crate::Relay;

/// RFC 6455 §1.3: appended to the client's key before hashing.
const WEBSOCKET_GUID: &[u8] = b"258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// Answer one request on the read port: `101` and a hand-off to the read side
/// for a WebSocket handshake, `426` for anything else.
pub(crate) async fn read(State(relay): State<Relay>, mut request: Request) -> Response {
    let Some(accept) = websocket_accept(request.method(), request.headers()) else {
        return upgrade_required();
    };
    // Present whenever the server driving this router supports upgrades;
    // absent when the router is called with no connection behind it.
    let Some(upgrade) = request.extensions_mut().remove::<OnUpgrade>() else {
        return upgrade_required();
    };
    // The peer is only a label on the framework's side. A server started
    // without connection info still serves reads.
    let peer = request
        .extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map_or(SocketAddr::from((Ipv4Addr::UNSPECIFIED, 0)), |info| info.0);

    // The stream exists only after the `101` below has been sent.
    tokio::spawn(async move {
        match upgrade.await {
            Ok(stream) => {
                if let Err(error) = relay.read_side.serve(TokioIo::new(stream), peer).await {
                    eprintln!("read: connection from {peer} ended: {error}");
                }
            }
            Err(error) => eprintln!("read: upgrade from {peer} failed: {error}"),
        }
    });

    (
        StatusCode::SWITCHING_PROTOCOLS,
        [
            (CONNECTION, "upgrade".to_string()),
            (UPGRADE, "websocket".to_string()),
            (SEC_WEBSOCKET_ACCEPT, accept),
        ],
    )
        .into_response()
}

fn upgrade_required() -> Response {
    (
        StatusCode::UPGRADE_REQUIRED,
        [(CONTENT_TYPE, "text/plain")],
        "Upgrade Required",
    )
        .into_response()
}

/// The `Sec-WebSocket-Accept` value for a request that is a WebSocket
/// handshake (RFC 6455 §4.2.1), or `None` if it is not one.
fn websocket_accept(method: &Method, headers: &HeaderMap) -> Option<String> {
    let key = headers.get(SEC_WEBSOCKET_KEY)?;
    let handshake = method == Method::GET
        && has_token(headers, &CONNECTION, "upgrade")
        && has_token(headers, &UPGRADE, "websocket")
        && has_token(headers, &SEC_WEBSOCKET_VERSION, "13");
    handshake.then(|| {
        let mut hash = Sha1::new();
        hash.update(key.as_bytes());
        hash.update(WEBSOCKET_GUID);
        STANDARD.encode(hash.finalize())
    })
}

/// Whether the comma-separated header `name` lists `token`, in any case.
fn has_token(headers: &HeaderMap, name: &HeaderName, token: &str) -> bool {
    headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .any(|listed| listed.trim().eq_ignore_ascii_case(token))
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    fn handshake() -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(CONNECTION, HeaderValue::from_static("Upgrade"));
        headers.insert(UPGRADE, HeaderValue::from_static("websocket"));
        headers.insert(SEC_WEBSOCKET_VERSION, HeaderValue::from_static("13"));
        headers.insert(
            SEC_WEBSOCKET_KEY,
            HeaderValue::from_static("dGhlIHNhbXBsZSBub25jZQ=="),
        );
        headers
    }

    #[test]
    fn the_accept_value_is_the_one_rfc_6455_works_out_for_its_sample_key() {
        assert_eq!(
            websocket_accept(&Method::GET, &handshake()).as_deref(),
            Some("s3pPLMBiTxaQ9kYGzzhZRbK+xOo=")
        );
    }

    #[test]
    fn a_connection_header_listing_upgrade_among_others_is_a_handshake() {
        let mut headers = handshake();
        headers.insert(CONNECTION, HeaderValue::from_static("keep-alive, Upgrade"));
        assert!(websocket_accept(&Method::GET, &headers).is_some());
    }

    #[test]
    fn a_request_missing_any_part_of_the_handshake_is_not_one() {
        for missing in [
            CONNECTION,
            UPGRADE,
            SEC_WEBSOCKET_VERSION,
            SEC_WEBSOCKET_KEY,
        ] {
            let mut headers = handshake();
            headers.remove(&missing);
            assert_eq!(websocket_accept(&Method::GET, &headers), None, "{missing}");
        }
        assert_eq!(websocket_accept(&Method::POST, &handshake()), None);

        let mut old = handshake();
        old.insert(SEC_WEBSOCKET_VERSION, HeaderValue::from_static("8"));
        assert_eq!(websocket_accept(&Method::GET, &old), None);
    }
}
