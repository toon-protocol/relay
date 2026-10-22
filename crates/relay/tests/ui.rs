//! The app UIs on the read port (#276), driven through the router with no
//! socket: what reaches an app, and what must still be the relay's own.
#![cfg(feature = "ui-repo")]

use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use relay::{Config, Relay};
use tempfile::TempDir;
use tower::ServiceExt;

fn relay(data: &TempDir, apps: Option<&str>) -> Relay {
    let data_dir = data.path().to_string_lossy().into_owned();
    let config = Config::from_env(|name| match name {
        "TOON_SECRET_KEY" => Some("1".repeat(64)),
        "TOON_DATA_DIR" => Some(data_dir.clone()),
        "TOON_UI_APPS" => apps.map(str::to_string),
        _ => None,
    })
    .expect("a complete configuration");
    Relay::open(&config).expect("an empty data directory opens")
}

async fn ask(
    apps: Option<&str>,
    method: Method,
    path: &str,
    accept: Option<&str>,
) -> axum::http::Response<Body> {
    let data = tempfile::tempdir().expect("a temp dir");
    let mut request = Request::builder().method(method).uri(path);
    if let Some(accept) = accept {
        request = request.header("accept", accept);
    }
    relay(&data, apps)
        .read_router()
        .oneshot(request.body(Body::empty()).expect("a request"))
        .await
        .expect("the router is infallible")
}

#[tokio::test]
async fn repo_is_a_page_with_the_policy_by_default() {
    let response = ask(None, Method::GET, "/repo", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .expect("text")
            .starts_with("text/html")
    );
    assert_eq!(
        response.headers()["content-security-policy"],
        "default-src 'self'"
    );
    assert_eq!(
        ask(None, Method::HEAD, "/repo", None).await.status(),
        StatusCode::OK
    );
}

#[tokio::test]
async fn the_root_and_other_methods_are_still_426() {
    assert_eq!(
        ask(None, Method::GET, "/", None).await.status(),
        StatusCode::UPGRADE_REQUIRED
    );
    assert_eq!(
        ask(None, Method::GET, "/repository", None).await.status(),
        StatusCode::UPGRADE_REQUIRED
    );
    assert_eq!(
        ask(None, Method::POST, "/repo", None).await.status(),
        StatusCode::UPGRADE_REQUIRED
    );
}

#[tokio::test]
async fn the_document_and_the_preflight_are_the_relays_on_a_ui_path() {
    let response = ask(None, Method::GET, "/repo", Some("application/nostr+json")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        response.headers()["content-type"]
            .to_str()
            .expect("text")
            .contains("application/nostr+json")
    );
    assert!(response.headers().get("content-security-policy").is_none());
    assert_eq!(
        ask(None, Method::OPTIONS, "/repo", None).await.status(),
        StatusCode::NO_CONTENT
    );
}

#[tokio::test]
async fn the_balance_asked_of_a_relay_selling_no_feed_is_still_426_on_a_ui_path() {
    assert_eq!(
        ask(
            None,
            Method::GET,
            "/repo",
            Some("application/toon-subscription+json")
        )
        .await
        .status(),
        StatusCode::UPGRADE_REQUIRED
    );
}

#[tokio::test]
async fn a_handshake_the_relay_refuses_is_still_426_on_a_ui_path() {
    let data = tempfile::tempdir().expect("a temp dir");
    let request = Request::builder()
        .method(Method::GET)
        .uri("/repo")
        .header("connection", "Upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .header("sec-websocket-version", "8")
        .body(Body::empty())
        .expect("a request");
    let response = relay(&data, None)
        .read_router()
        .oneshot(request)
        .await
        .expect("the router is infallible");
    assert_eq!(response.status(), StatusCode::UPGRADE_REQUIRED);
}

#[tokio::test]
async fn an_app_switched_off_returns_its_prefix_to_426() {
    for apps in ["", " , "] {
        assert_eq!(
            ask(Some(apps), Method::GET, "/repo", None).await.status(),
            StatusCode::UPGRADE_REQUIRED,
            "{apps:?}"
        );
    }
    assert_eq!(
        ask(Some("repo"), Method::GET, "/repo", None).await.status(),
        StatusCode::OK
    );
}

#[test]
fn an_app_no_one_wrote_is_refused() {
    let data = tempfile::tempdir().expect("a temp dir");
    let data_dir = data.path().to_string_lossy().into_owned();
    let refused = Config::from_env(|name| match name {
        "TOON_SECRET_KEY" => Some("1".repeat(64)),
        "TOON_DATA_DIR" => Some(data_dir.clone()),
        "TOON_UI_APPS" => Some("repo,nope".to_string()),
        _ => None,
    });
    assert!(refused.is_err());
}
