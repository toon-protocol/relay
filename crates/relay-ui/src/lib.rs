//! App UIs the relay serves on its read port (#276).
//!
//! An app UI is a mount prefix and an axum [`Router`], built from a
//! [`ReadOnlyStore`]: the one view of the relay's events an app gets. It can
//! ask the store a [`Filter`] and read what comes back; it cannot write, and
//! it cannot see how events are kept. The relay decides which apps are on and
//! hands each its handle; this crate holds no relay code.

mod assets;

pub use assets::{Assets, CACHE_CONTROL_VALUE, MANIFEST_FILE};

use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::extract::Request;
use axum::http::header::CONTENT_SECURITY_POLICY;
use axum::http::{HeaderValue, Response};
use nostr::event::Event;
use nostr::filter::Filter;
use tower::ServiceExt;

/// What an app's pages may load unless the page sets its own policy: only
/// their own origin, and no inline script (an inline script is not `'self'`).
/// `'wasm-unsafe-eval'` lets the page compile its own WebAssembly, which a
/// bare `default-src` forbids; it does not allow `eval` of script text.
pub const CONTENT_SECURITY_POLICY_VALUE: &str =
    "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'";

/// What a store can be asked, and nothing else. The relay implements it for
/// its store; it has no method that writes.
pub trait EventSource: Send + Sync + 'static {
    /// The stored events matching `filter`, newest first, at most its
    /// `limit`. A store that cannot answer returns no events.
    fn query(&self, filter: Filter) -> Pin<Box<dyn Future<Output = Vec<Event>> + Send + '_>>;
}

/// The read-only query handle an app UI is built from. Cheap to clone.
#[derive(Clone)]
pub struct ReadOnlyStore {
    source: Arc<dyn EventSource>,
}

impl std::fmt::Debug for ReadOnlyStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ReadOnlyStore")
            .finish_non_exhaustive()
    }
}

impl ReadOnlyStore {
    /// A handle that asks `source`.
    pub fn new(source: impl EventSource) -> Self {
        Self {
            source: Arc::new(source),
        }
    }

    /// The stored events matching `filter`, newest first.
    pub async fn query(&self, filter: Filter) -> Vec<Event> {
        self.source.query(filter).await
    }
}

/// An app UI: pages under a mount prefix.
#[derive(Debug, Clone)]
pub struct AppUi {
    prefix: &'static str,
    router: Router,
}

impl AppUi {
    /// An app UI at `prefix` (`/repo`: a leading slash, no trailing one),
    /// whose `router` sees paths as the client sent them, prefix included.
    ///
    /// # Panics
    ///
    /// If `prefix` is not a slash and a name: a mount that could shadow the
    /// whole read port is a programming error, not a setting.
    pub fn new(prefix: &'static str, router: Router) -> Self {
        assert!(
            prefix.len() > 1 && prefix.starts_with('/') && !prefix.ends_with('/'),
            "an app UI prefix is a slash and a name, like /repo: {prefix:?}"
        );
        Self { prefix, router }
    }

    /// The mount prefix.
    pub fn prefix(&self) -> &'static str {
        self.prefix
    }

    /// Whether `path` is the prefix itself or below it (`/repo` and
    /// `/repo/x`, not `/repository`).
    pub fn serves(&self, path: &str) -> bool {
        path.strip_prefix(self.prefix)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }

    /// Answer a request under the prefix. Every answer carries a
    /// Content-Security-Policy: the one the page set for itself (a page that
    /// fetches from elsewhere widens `connect-src`), else
    /// [`CONTENT_SECURITY_POLICY_VALUE`].
    pub async fn respond(&self, request: Request) -> Response<Body> {
        let Ok(mut response) = self
            .router
            .clone()
            .oneshot(request)
            .await
            .map(axum::response::IntoResponse::into_response)
            .map_err(|never: Infallible| never);
        response
            .headers_mut()
            .entry(CONTENT_SECURITY_POLICY)
            .or_insert(HeaderValue::from_static(CONTENT_SECURITY_POLICY_VALUE));
        response
    }
}

#[cfg(test)]
mod tests {
    use axum::http::{Method, StatusCode};
    use axum::routing::get;

    use super::*;

    fn app() -> AppUi {
        AppUi::new(
            "/repo",
            Router::new().route("/repo", get(|| async { "page" })),
        )
    }

    #[test]
    fn a_prefix_serves_itself_and_what_is_below_it_only() {
        let app = app();
        assert!(app.serves("/repo"));
        assert!(app.serves("/repo/x/y"));
        assert!(!app.serves("/repository"));
        assert!(!app.serves("/"));
        assert!(!app.serves("/other/repo"));
    }

    #[tokio::test]
    async fn every_answer_carries_the_policy() {
        let app = app();
        for path in ["/repo", "/repo/missing"] {
            let request = Request::builder()
                .method(Method::GET)
                .uri(path)
                .body(Body::empty())
                .unwrap();
            let response = app.respond(request).await;
            assert_eq!(
                response.headers()[CONTENT_SECURITY_POLICY],
                CONTENT_SECURITY_POLICY_VALUE,
                "{path}"
            );
        }
        let request = Request::builder().uri("/repo").body(Body::empty()).unwrap();
        assert_eq!(app.respond(request).await.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn a_page_may_set_its_own_policy() {
        let app = AppUi::new(
            "/repo",
            Router::new().route(
                "/repo",
                get(|| async { ([(CONTENT_SECURITY_POLICY, "default-src 'self'")], "page") }),
            ),
        );
        let request = Request::builder().uri("/repo").body(Body::empty()).unwrap();
        let response = app.respond(request).await;
        assert_eq!(
            response
                .headers()
                .get_all(CONTENT_SECURITY_POLICY)
                .iter()
                .count(),
            1
        );
        assert_eq!(
            response.headers()[CONTENT_SECURITY_POLICY],
            "default-src 'self'"
        );
    }
}
