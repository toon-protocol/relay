//! The built UI assets an app serves (#283): a folder of content-hashed files
//! and a `manifest.json` mapping each logical name to the hashed one.
//!
//! `cargo xtask ui-assets` writes the folder; the relay loads it once at
//! start from `TOON_UI_ASSETS_DIR`. Only files the manifest names are served,
//! so a request path is never joined onto the filesystem.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use axum::body::{Body, Bytes};
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, Response, StatusCode};
use axum::response::IntoResponse;

/// The manifest's file name inside the assets folder.
pub const MANIFEST_FILE: &str = "manifest.json";

/// What a hashed file may be cached as: its name changes when its bytes do.
pub const CACHE_CONTROL_VALUE: &str = "public, max-age=31536000, immutable";

/// The loaded assets. Cheap to clone.
#[derive(Debug, Clone)]
pub struct Assets {
    /// Logical name to hashed file name.
    manifest: Arc<BTreeMap<String, String>>,
    /// Hashed file name to its bytes.
    files: Arc<BTreeMap<String, Bytes>>,
}

impl Assets {
    /// Load the folder `dir`. A missing or unreadable folder, manifest or
    /// file, or a manifest that names a path rather than a file, gives
    /// `None`: the relay still starts, and the pages that need assets say
    /// they are unavailable.
    pub fn load(dir: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(dir.join(MANIFEST_FILE)).ok()?;
        let manifest: BTreeMap<String, String> = serde_json::from_str(&text).ok()?;
        let mut files = BTreeMap::new();
        for hashed in manifest.values() {
            if hashed.is_empty() || hashed.contains(['/', '\\']) || hashed.starts_with('.') {
                return None;
            }
            let bytes = std::fs::read(dir.join(hashed)).ok()?;
            files.insert(hashed.clone(), Bytes::from(bytes));
        }
        Some(Self {
            manifest: Arc::new(manifest),
            files: Arc::new(files),
        })
    }

    /// The URL of the asset called `logical`, under `prefix` (the app's
    /// mount, like `/repo`), if the manifest has it.
    pub fn url(&self, prefix: &str, logical: &str) -> Option<String> {
        self.manifest
            .get(logical)
            .map(|hashed| format!("{prefix}/assets/{hashed}"))
    }

    /// The immutable answer for the hashed file `name`, if there is one.
    pub fn respond(&self, name: &str) -> Response<Body> {
        let Some(bytes) = self.files.get(name) else {
            return StatusCode::NOT_FOUND.into_response();
        };
        let mut response = Response::new(Body::from(bytes.clone()));
        let headers = response.headers_mut();
        headers.insert(CONTENT_TYPE, HeaderValue::from_static(content_type(name)));
        headers.insert(CACHE_CONTROL, HeaderValue::from_static(CACHE_CONTROL_VALUE));
        response
    }
}

/// The MIME type for a file name's extension.
fn content_type(name: &str) -> &'static str {
    match name.rsplit_once('.').map(|(_, extension)| extension) {
        Some("wasm") => "application/wasm",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("html") => "text/html; charset=utf-8",
        _ => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(manifest: &str, files: &[(&str, &[u8])]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(MANIFEST_FILE), manifest).unwrap();
        for (name, bytes) in files {
            std::fs::write(dir.path().join(name), bytes).unwrap();
        }
        dir
    }

    #[test]
    fn a_folder_loads_and_serves_hashed_files_as_immutable() {
        let dir = folder(
            r#"{"app.wasm":"app-abc.wasm","app.js":"app-def.js"}"#,
            &[("app-abc.wasm", b"\0asm"), ("app-def.js", b"x")],
        );
        let assets = Assets::load(dir.path()).expect("loads");
        assert_eq!(
            assets.url("/repo", "app.wasm").as_deref(),
            Some("/repo/assets/app-abc.wasm")
        );
        assert_eq!(assets.url("/repo", "other"), None);
        let response = assets.respond("app-abc.wasm");
        assert_eq!(response.headers()[CONTENT_TYPE], "application/wasm");
        assert_eq!(response.headers()[CACHE_CONTROL], CACHE_CONTROL_VALUE);
        assert_eq!(
            assets.respond("manifest.json").status(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn a_missing_or_broken_folder_is_no_assets() {
        assert!(Assets::load(Path::new("/nonexistent/ui-assets")).is_none());
        let empty = tempfile::tempdir().unwrap();
        assert!(Assets::load(empty.path()).is_none());
        let broken = folder("not json", &[]);
        assert!(Assets::load(broken.path()).is_none());
        let missing_file = folder(r#"{"a":"a-1.js"}"#, &[]);
        assert!(Assets::load(missing_file.path()).is_none());
        let traversal = folder(r#"{"a":"../a.js"}"#, &[]);
        assert!(Assets::load(traversal.path()).is_none());
    }
}
