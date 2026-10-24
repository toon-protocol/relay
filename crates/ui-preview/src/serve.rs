//! Cards served over HTTP at bounded cost. A crawler that unfurls a link
//! fetches its image, and may fetch every repo's: so a drawn card is kept in
//! a cache of a bounded number of entries and bytes, least recently used out
//! first, under its [`Card::key`]; a card is drawn on a blocking thread, at
//! most [`MAX_DRAWING`] at once, and a request past that is told to come
//! back rather than queued. The key is the response's ETag, so a crawler
//! that asks again with `If-None-Match` is answered `304` without a draw.

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use axum::body::Body;
use axum::http::header::{
    CACHE_CONTROL, CONTENT_TYPE, ETAG, IF_NONE_MATCH, RETRY_AFTER, X_CONTENT_TYPE_OPTIONS,
};
use axum::http::{HeaderMap, HeaderValue, Response, StatusCode};

use crate::Card;

/// How long a client or a crawler may keep a card before asking again.
pub const CACHE_CONTROL_VALUE: &str = "public, max-age=600";
/// The most cards kept.
const MAX_ENTRIES: usize = 512;
/// The most bytes of cards kept.
const MAX_BYTES: usize = 32 * 1024 * 1024;
/// The most cards drawn at once, across every request.
const MAX_DRAWING: usize = 2;

/// Drawn cards, and how many are being drawn. Cheap to clone: clones share
/// both.
#[derive(Clone, Default)]
pub struct Previews {
    cache: Arc<Mutex<Cache>>,
    drawing: Arc<AtomicUsize>,
}

impl std::fmt::Debug for Previews {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Previews").finish_non_exhaustive()
    }
}

impl Previews {
    /// An empty cache.
    pub fn new() -> Self {
        Self::default()
    }

    /// The answer to a request for `card` with `headers`: `404` when there
    /// is no such card, `304` when the request already holds it, else the
    /// PNG, from the cache or drawn now, or `503` while too many are being
    /// drawn.
    pub async fn respond(&self, headers: &HeaderMap, card: Option<Card>) -> Response<Body> {
        let Some(card) = card else {
            return plain(StatusCode::NOT_FOUND, "no such card");
        };
        let key = card.key();
        let etag = format!("\"{key}\"");
        if holds(headers, &etag) {
            return png(StatusCode::NOT_MODIFIED, &etag, Body::empty());
        }
        if let Some(bytes) = self.lock().get(&key) {
            return png(StatusCode::OK, &etag, Body::from(bytes.to_vec()));
        }
        let Some(_turn) = Turn::take(&self.drawing) else {
            let mut response = plain(
                StatusCode::SERVICE_UNAVAILABLE,
                "drawing other cards; try again",
            );
            response
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from_static("2"));
            return response;
        };
        match tokio::task::spawn_blocking(move || card.png()).await {
            Ok(Ok(bytes)) => {
                let bytes: Arc<[u8]> = bytes.into();
                self.lock().put(key, bytes.clone());
                png(StatusCode::OK, &etag, Body::from(bytes.to_vec()))
            }
            Ok(Err(error)) => {
                eprintln!("ui: a preview card was not drawn: {error}");
                plain(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "the card could not be drawn",
                )
            }
            Err(error) => {
                eprintln!("ui: drawing a preview card stopped: {error}");
                plain(
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "the card could not be drawn",
                )
            }
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Cache> {
        // A panic while the lock was held leaves the cache as it was: a map
        // of finished entries, still sound to read.
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// How many cards are kept, and their bytes.
    pub fn held(&self) -> (usize, usize) {
        let cache = self.lock();
        (cache.entries.len(), cache.bytes)
    }
}

/// Whether `headers` say the client holds `etag`.
fn holds(headers: &HeaderMap, etag: &str) -> bool {
    headers
        .get_all(IF_NONE_MATCH)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .map(|tag| tag.trim().trim_start_matches("W/"))
        .any(|tag| tag == etag || tag == "*")
}

fn png(status: StatusCode, etag: &str, body: Body) -> Response<Body> {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    let headers = response.headers_mut();
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("image/png"));
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(CACHE_CONTROL_VALUE));
    headers.insert(X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff"));
    if let Ok(etag) = HeaderValue::from_str(etag) {
        headers.insert(ETAG, etag);
    }
    response
}

fn plain(status: StatusCode, text: &'static str) -> Response<Body> {
    let mut response = Response::new(Body::from(text));
    *response.status_mut() = status;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain; charset=utf-8"),
    );
    response
}

/// One of the [`MAX_DRAWING`] turns to draw, given back when dropped.
struct Turn<'a>(&'a AtomicUsize);

impl<'a> Turn<'a> {
    fn take(drawing: &'a AtomicUsize) -> Option<Self> {
        drawing
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |now| {
                (now < MAX_DRAWING).then_some(now + 1)
            })
            .ok()
            .map(|_| Self(drawing))
    }
}

impl Drop for Turn<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Cards by key, least recently used out first once there are more than
/// [`MAX_ENTRIES`] or [`MAX_BYTES`] of them.
#[derive(Default)]
struct Cache {
    entries: HashMap<String, (Arc<[u8]>, u64)>,
    bytes: usize,
    clock: u64,
}

impl Cache {
    fn get(&mut self, key: &str) -> Option<Arc<[u8]>> {
        self.clock += 1;
        let clock = self.clock;
        self.entries.get_mut(key).map(|(bytes, used)| {
            *used = clock;
            bytes.clone()
        })
    }

    fn put(&mut self, key: String, bytes: Arc<[u8]>) {
        if bytes.len() > MAX_BYTES {
            return;
        }
        self.clock += 1;
        self.bytes += bytes.len();
        if let Some((old, _)) = self.entries.insert(key, (bytes, self.clock)) {
            self.bytes -= old.len();
        }
        while self.entries.len() > MAX_ENTRIES || self.bytes > MAX_BYTES {
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, (_, used))| *used)
                .map(|(key, _)| key.clone())
            else {
                break;
            };
            if let Some((gone, _)) = self.entries.remove(&oldest) {
                self.bytes -= gone.len();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cache_keeps_the_most_recently_used_within_its_bounds() {
        let mut cache = Cache::default();
        for n in 0..MAX_ENTRIES {
            cache.put(n.to_string(), vec![0u8; 10].into());
        }
        assert!(cache.get("0").is_some());
        cache.put("new".into(), vec![0u8; 10].into());
        assert_eq!(cache.entries.len(), MAX_ENTRIES);
        assert!(cache.get("0").is_some(), "used recently, so kept");
        assert!(cache.get("1").is_none(), "the least recently used went");
        assert_eq!(cache.bytes, MAX_ENTRIES * 10);

        cache.put("big".into(), vec![0u8; MAX_BYTES - 100].into());
        assert!(cache.bytes <= MAX_BYTES);
        assert!(cache.get("big").is_some());
        cache.put("huge".into(), vec![0u8; MAX_BYTES + 1].into());
        assert!(cache.get("huge").is_none());
    }

    #[test]
    fn only_so_many_turns_to_draw_are_given_at_once() {
        let drawing = AtomicUsize::new(0);
        let turns: Vec<Turn<'_>> = (0..MAX_DRAWING)
            .filter_map(|_| Turn::take(&drawing))
            .collect();
        assert_eq!(turns.len(), MAX_DRAWING);
        assert!(Turn::take(&drawing).is_none());
        drop(turns);
        assert!(Turn::take(&drawing).is_some());
        assert_eq!(drawing.load(Ordering::Acquire), 0);
    }

    #[test]
    fn a_client_holding_the_etag_is_told_so() {
        let mut headers = HeaderMap::new();
        headers.insert(IF_NONE_MATCH, HeaderValue::from_static("\"a\", W/\"b\""));
        assert!(holds(&headers, "\"a\""));
        assert!(holds(&headers, "\"b\""));
        assert!(!holds(&headers, "\"c\""));
        assert!(!holds(&HeaderMap::new(), "\"a\""));
    }
}
