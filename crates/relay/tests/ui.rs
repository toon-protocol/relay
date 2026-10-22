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
        "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'"
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

#[test]
fn object_sources_must_be_https_urls() {
    let data = tempfile::tempdir().expect("a temp dir");
    let data_dir = data.path().to_string_lossy().into_owned();
    let with = |setting: &'static str, value: &'static str| {
        Config::from_env(|name| match name {
            "TOON_SECRET_KEY" => Some("1".repeat(64)),
            "TOON_DATA_DIR" => Some(data_dir.clone()),
            other if other == setting => Some(value.to_string()),
            _ => None,
        })
    };
    for setting in ["TOON_UI_ARWEAVE_GATEWAYS", "TOON_UI_BLOB_SERVERS"] {
        assert!(with(setting, "https://a.test, https://b.test/x").is_ok());
        for bad in [
            "http://a.test",
            "a.test",
            "https://",
            "https://a.test,ftp://b",
        ] {
            assert!(with(setting, bad).is_err(), "{setting}={bad}");
        }
    }
}

async fn body_text(response: axum::http::Response<Body>) -> String {
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("a body");
    String::from_utf8(bytes.to_vec()).expect("text")
}

#[tokio::test]
async fn with_no_assets_the_page_says_browsing_is_unavailable() {
    let response = ask(None, Method::GET, "/repo", None).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        body_text(response)
            .await
            .contains("git browsing unavailable")
    );
    assert_eq!(
        ask(None, Method::GET, "/repo/assets/x.wasm", None)
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn assets_in_the_configured_folder_are_served_hashed_and_immutable() {
    let data = tempfile::tempdir().expect("a temp dir");
    let assets = tempfile::tempdir().expect("a temp dir");
    std::fs::write(
        assets.path().join("manifest.json"),
        r#"{"ui-nip34-web.js":"boot.abc.js","ui-nip34-web.wasm":"m.abc.wasm"}"#,
    )
    .expect("manifest");
    std::fs::write(assets.path().join("boot.abc.js"), "//").expect("js");
    std::fs::write(assets.path().join("m.abc.wasm"), b"\0asm").expect("wasm");
    let data_dir = data.path().to_string_lossy().into_owned();
    let assets_dir = assets.path().to_string_lossy().into_owned();
    let config = Config::from_env(|name| match name {
        "TOON_SECRET_KEY" => Some("1".repeat(64)),
        "TOON_DATA_DIR" => Some(data_dir.clone()),
        "TOON_UI_ASSETS_DIR" => Some(assets_dir.clone()),
        _ => None,
    })
    .expect("a complete configuration");
    let router = Relay::open(&config).expect("opens").read_router();
    let get = |path: &'static str| {
        let router = router.clone();
        async move {
            router
                .oneshot(
                    Request::builder()
                        .uri(path)
                        .body(Body::empty())
                        .expect("a request"),
                )
                .await
                .expect("infallible")
        }
    };

    let page = body_text(get("/repo").await).await;
    assert!(page.contains("/repo/assets/boot.abc.js"), "{page}");
    assert!(!page.contains("unavailable"));

    let wasm = get("/repo/assets/m.abc.wasm").await;
    assert_eq!(wasm.status(), StatusCode::OK);
    assert_eq!(wasm.headers()["content-type"], "application/wasm");
    assert_eq!(
        wasm.headers()["cache-control"],
        "public, max-age=31536000, immutable"
    );
    assert_eq!(
        get("/repo/assets/manifest.json").await.status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_folder_without_a_manifest_still_starts_the_relay() {
    let data = tempfile::tempdir().expect("a temp dir");
    let data_dir = data.path().to_string_lossy().into_owned();
    let config = Config::from_env(|name| match name {
        "TOON_SECRET_KEY" => Some("1".repeat(64)),
        "TOON_DATA_DIR" => Some(data_dir.clone()),
        "TOON_UI_ASSETS_DIR" => Some("/nonexistent".to_string()),
        _ => None,
    })
    .expect("a complete configuration");
    let response = Relay::open(&config)
        .expect("opens")
        .read_router()
        .oneshot(
            Request::builder()
                .uri("/repo")
                .body(Body::empty())
                .expect("a request"),
        )
        .await
        .expect("infallible");
    assert!(
        body_text(response)
            .await
            .contains("git browsing unavailable")
    );
}
