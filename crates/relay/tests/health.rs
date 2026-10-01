//! `GET /health` on the write port, driven through the router with no socket.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use relay::{Config, Relay};
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt;

/// The x-only public key of the secret key `11…11`.
const PUBKEY_OF_ONES: &str = "4f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa";

fn relay(data: &TempDir) -> Relay {
    let data_dir = data.path().to_string_lossy().into_owned();
    let config = Config::from_env(|name| match name {
        "TOON_SECRET_KEY" => Some("1".repeat(64)),
        "TOON_DATA_DIR" => Some(data_dir.clone()),
        _ => None,
    })
    .expect("a secret key and a data directory are a complete configuration");
    Relay::open(&config).expect("an empty data directory opens")
}

async fn get(path: &str) -> (StatusCode, Vec<u8>) {
    let data = tempfile::tempdir().expect("a temp dir");
    let response = relay(&data)
        .write_router()
        .oneshot(
            Request::get(path)
                .body(Body::empty())
                .expect("a GET with an empty body is a valid request"),
        )
        .await
        .expect("the router is infallible");
    let status = response.status();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("an in-memory body cannot fail to collect")
        .to_bytes();
    (status, body.to_vec())
}

fn now_ms() -> u64 {
    let since_epoch = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the test host's clock is after 1970");
    u64::try_from(since_epoch.as_millis()).expect("milliseconds since 1970 fit in 64 bits")
}

#[tokio::test]
async fn health_reports_liveness_the_identity_and_the_version() {
    let before = now_ms();
    let (status, body) = get("/health").await;
    let after = now_ms();

    assert_eq!(status, StatusCode::OK);
    let body: Value = serde_json::from_slice(&body).expect("the health body is JSON");
    let object = body.as_object().expect("the health body is an object");
    let mut keys: Vec<_> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        ["capabilities", "pubkey", "status", "timestamp", "version"],
        "exactly the documented keys"
    );
    assert_eq!(body["status"], "healthy");
    assert_eq!(body["pubkey"], PUBKEY_OF_ONES);
    assert_eq!(body["capabilities"], serde_json::json!(["relay"]));
    assert_eq!(body["version"], env!("CARGO_PKG_VERSION"));
    let timestamp = body["timestamp"]
        .as_u64()
        .expect("the timestamp is a whole number of milliseconds");
    assert!((before..=after).contains(&timestamp));
}
