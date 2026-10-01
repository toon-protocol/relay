//! The Relay Information Document on the read port, driven through the router
//! with no socket. A relay with no connector publishes no edge.

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use http_body_util::BodyExt;
use relay::{Config, Relay};
use serde_json::{Value, json};
use tempfile::TempDir;
use tower::ServiceExt;

fn relay(data: &TempDir) -> Relay {
    let data_dir = data.path().to_string_lossy().into_owned();
    let config = Config::from_env(|name| match name {
        "TOON_SECRET_KEY" => Some("1".repeat(64)),
        "TOON_DATA_DIR" => Some(data_dir.clone()),
        "TOON_RELAY_NAME" => Some("devnet".to_string()),
        _ => None,
    })
    .expect("a complete configuration");
    Relay::open(&config).expect("an empty data directory opens")
}

async fn ask(method: Method, accept: Option<&str>) -> (StatusCode, axum::http::HeaderMap, Vec<u8>) {
    let data = tempfile::tempdir().expect("a temp dir");
    let mut request = Request::builder().method(method).uri("/");
    if let Some(accept) = accept {
        request = request.header("accept", accept);
    }
    let response = relay(&data)
        .read_router()
        .oneshot(request.body(Body::empty()).expect("a request"))
        .await
        .expect("the router is infallible");
    let (parts, body) = response.into_parts();
    let body = body.collect().await.expect("a body").to_bytes();
    (parts.status, parts.headers, body.to_vec())
}

#[tokio::test]
async fn asking_for_the_document_by_name_gets_it_without_an_edge() {
    let (status, headers, body) =
        ask(Method::GET, Some("text/html, Application/Nostr+JSON;q=0.9")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers["content-type"]
            .to_str()
            .expect("text")
            .contains("application/nostr+json")
    );
    assert_eq!(headers["access-control-allow-origin"], "*");
    let document: Value = serde_json::from_slice(&body).expect("JSON");
    assert_eq!(document["name"], "devnet");
    assert_eq!(document["supported_nips"], json!([1, 9, 11, 16, 40]));
    assert_eq!(document["limitation"]["payment_required"], json!(false));
    assert_eq!(document["limitation"]["restricted_writes"], json!(true));
    assert!(document.get("toon").is_none());
}

#[tokio::test]
async fn every_other_request_is_still_upgrade_required() {
    for accept in [None, Some("text/html"), Some("application/json")] {
        let (status, _, body) = ask(Method::GET, accept).await;
        assert_eq!(status, StatusCode::UPGRADE_REQUIRED);
        assert_eq!(body, b"Upgrade Required");
    }
}

#[tokio::test]
async fn a_preflight_is_answered_204_with_the_cors_headers() {
    let (status, headers, _) = ask(Method::OPTIONS, None).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(headers["access-control-allow-origin"], "*");
    assert!(
        headers["access-control-allow-methods"]
            .to_str()
            .expect("text")
            .contains("GET")
    );
    assert!(
        headers["access-control-allow-headers"]
            .to_str()
            .expect("text")
            .contains("accept")
    );
}
