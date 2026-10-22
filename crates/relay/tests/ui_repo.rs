//! The `/repo` pages over events written through `POST /write` (#282),
//! driven through the router with no socket.
#![cfg(feature = "ui-repo")]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{delivery, running, signed_by, write};
use http_body_util::BodyExt;
use nostr::event::Event;
use nostr::key::Keys;
use nostr::nips::nip19::{Nip19Coordinate, ToBech32};
use tower::ServiceExt;

async fn get(relay: &relay::Relay, path: &str) -> (StatusCode, Option<String>, String) {
    let response = relay
        .read_router()
        .oneshot(Request::get(path).body(Body::empty()).expect("a request"))
        .await
        .expect("infallible");
    let status = response.status();
    let location = response
        .headers()
        .get("location")
        .map(|value| value.to_str().expect("text").to_owned());
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (
        status,
        location,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

async fn store(relay: &relay::Relay, event: &Event) {
    assert_eq!(write(relay, delivery(event)).await.0, 200);
}

#[tokio::test]
async fn the_pages_read_written_events_and_ignore_a_strangers_refs() {
    let running = running().await;
    let relay = &running.relay;
    let owner = Keys::generate();
    let maintainer = Keys::generate();
    let stranger = Keys::generate();
    let owner_hex = owner.public_key().to_hex();
    let announcement = signed_by(
        &owner,
        30_617,
        10,
        &[
            &["d", "proj"],
            &["name", "Project <b>"],
            &["description", "A <script>alert(1)</script> repo"],
            &["maintainers", &maintainer.public_key().to_hex()],
            &[
                "clone",
                "https://example.test/proj.git",
                "javascript:alert(1)",
            ],
        ],
    );
    store(relay, &announcement).await;
    store(
        relay,
        &signed_by(
            &maintainer,
            30_618,
            20,
            &[
                &["d", "proj"],
                &["HEAD", "ref: refs/heads/main"],
                &["refs/heads/main", &"a".repeat(40)],
            ],
        ),
    )
    .await;
    store(
        relay,
        &signed_by(
            &stranger,
            30_618,
            30,
            &[&["d", "proj"], &["refs/heads/evil", &"b".repeat(40)]],
        ),
    )
    .await;
    let issue = signed_by(
        &stranger,
        1621,
        40,
        &[&["a", &format!("30617:{owner_hex}:proj")]],
    );
    store(relay, &issue).await;

    let (status, _, list) = get(relay, "/repo").await;
    assert_eq!(status, StatusCode::OK);
    assert!(list.contains("Project &lt;b&gt;"), "{list}");
    assert!(!list.contains("<script"), "{list}");

    let home = format!("/repo/{}/proj", owner.public_key().to_bech32().unwrap());
    let (status, _, page) = get(relay, &home).await;
    assert_eq!(status, StatusCode::OK);
    assert!(page.contains("refs/heads/main"), "{page}");
    assert!(page.contains("Default branch: <code>main</code>"), "{page}");
    assert!(page.contains(&maintainer.public_key().to_bech32().unwrap()));
    assert!(page.contains("<a href=\"https://example.test/proj.git\""));
    assert!(!page.contains("href=\"javascript:"), "{page}");
    assert!(!page.contains("evil"), "{page}");
    assert!(!page.contains("<script"), "{page}");

    let naddr = Nip19Coordinate::new(
        nostr::nips::nip01::Coordinate::new(nostr::event::Kind::from(30_617), owner.public_key())
            .identifier("proj"),
        [],
    )
    .to_bech32()
    .unwrap();
    for target in [naddr, announcement.id.to_hex(), issue.id.to_hex()] {
        let (status, location, _) = get(relay, &format!("/repo/{target}")).await;
        assert_eq!(status, StatusCode::MOVED_PERMANENTLY, "{target}");
        assert_eq!(location.as_deref(), Some(home.as_str()));
    }
}

#[tokio::test]
async fn an_unknown_repo_or_id_is_a_404_page() {
    let running = running().await;
    let relay = &running.relay;
    let owner = Keys::generate();
    for path in [
        format!("/repo/{}", "0".repeat(64)),
        format!("/repo/{}/nope", owner.public_key().to_bech32().unwrap()),
        "/repo/a/b/c".to_owned(),
    ] {
        let (status, _, page) = get(relay, &path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}");
        assert!(page.starts_with("<!doctype html>"), "{path}");
        assert!(!page.contains("<script"), "{path}");
    }
}

/// The tree and blob pages: the layout the browser fills, in the order it
/// tries things, and nothing fetched by the relay (#285).
#[tokio::test]
async fn git_pages_carry_ordered_candidates_and_their_own_policy() {
    let assets = tempfile::tempdir().expect("a temp dir");
    std::fs::write(
        assets.path().join("manifest.json"),
        r#"{"ui-nip34-web.js":"boot.abc.js"}"#,
    )
    .expect("manifest");
    std::fs::write(assets.path().join("boot.abc.js"), "//").expect("js");
    let assets_dir = assets.path().to_string_lossy().into_owned();
    let running = common::running_with(|name| match name {
        "TOON_UI_ASSETS_DIR" => Some(assets_dir.clone()),
        "TOON_UI_ARWEAVE_GATEWAYS" => Some("https://operator-gateway.test".to_owned()),
        "TOON_UI_BLOB_SERVERS" => Some("https://operator-blossom.test".to_owned()),
        _ => None,
    })
    .await;
    let relay = &running.relay;
    let owner = Keys::generate();
    let commit = "a".repeat(40);
    let blob = "c".repeat(64);
    store(
        relay,
        &signed_by(
            &owner,
            30_617,
            10,
            &[&["d", "proj"], &["blossoms", "https://own-blossom.test"]],
        ),
    )
    .await;
    store(
        relay,
        &signed_by(
            &owner,
            30_618,
            20,
            &[
                &["d", "proj"],
                &["refs/heads/feature/x", &commit],
                &["arweave", &commit, "tx1", "https://own-gateway.test"],
                &["blossom", &commit, &blob],
            ],
        ),
    )
    .await;
    let home = format!("/repo/{}/proj", owner.public_key().to_bech32().unwrap());
    for (page, kind, path) in [
        ("tree", "tree", "feature/x"),
        ("blob", "blob", "feature/x/dir/a%20b.txt"),
    ] {
        let response = relay
            .read_router()
            .oneshot(
                Request::get(format!("{home}/{page}/{path}"))
                    .body(Body::empty())
                    .expect("a request"),
            )
            .await
            .expect("infallible");
        assert_eq!(response.status(), StatusCode::OK, "{page}");
        assert_eq!(
            response.headers()["content-security-policy"],
            "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src https:"
        );
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("body")
            .to_bytes();
        let html = String::from_utf8_lossy(&bytes);
        assert!(
            html.contains("<script type=\"module\" src=\"/repo/assets/boot.abc.js\">"),
            "{html}"
        );
        assert_eq!(html.matches("<script").count(), 1, "{html}");
        let layout = html
            .split("data-git=\"")
            .nth(1)
            .and_then(|rest| rest.split('"').next())
            .expect("a layout")
            .replace("&quot;", "\"");
        let layout: serde_json::Value = serde_json::from_str(&layout).expect("json");
        assert_eq!(layout["kind"], kind);
        assert_eq!(layout["commit"], commit);
        assert_eq!(layout["ref"], "feature/x");
        assert_eq!(layout["base"], home);
        let expected_path: &[&str] = if page == "blob" {
            &["dir", "a b.txt"]
        } else {
            &[]
        };
        assert_eq!(layout["path"], serde_json::json!(expected_path));
        assert_eq!(
            layout["objects"][&commit],
            serde_json::json!([
                "https://own-gateway.test/tx1",
                format!("https://own-blossom.test/{blob}"),
                "https://operator-gateway.test/tx1",
                format!("https://operator-blossom.test/{blob}"),
            ])
        );
    }
    for missing in [
        format!("{home}/blob/nope/a"),
        format!("{home}/blob/feature/x"),
    ] {
        let (status, _, _) = get(relay, &missing).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{missing}");
    }
}
