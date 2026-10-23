//! The app UIs' stylesheet: rig-web's look, compiled into the binary so a
//! page is styled whether or not the relay has its built assets.

use std::sync::OnceLock;

use axum::body::Body;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, Response};
use sha2::{Digest, Sha256};

use crate::CACHE_CONTROL_VALUE;

/// The stylesheet's text.
pub const STYLESHEET: &str = include_str!("style.css");

/// The stylesheet's file name, `style.<hash>.css`: it changes when the text
/// does, so it can be cached as immutable like the built assets.
pub fn stylesheet_name() -> &'static str {
    static NAME: OnceLock<String> = OnceLock::new();
    NAME.get_or_init(|| {
        let digest = Sha256::digest(STYLESHEET.as_bytes());
        let hash: String = digest[..8].iter().map(|b| format!("{b:02x}")).collect();
        format!("style.{hash}.css")
    })
}

/// The immutable answer for the stylesheet.
pub fn stylesheet_response() -> Response<Body> {
    let mut response = Response::new(Body::from(STYLESHEET));
    let headers = response.headers_mut();
    headers.insert(
        CONTENT_TYPE,
        HeaderValue::from_static("text/css; charset=utf-8"),
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static(CACHE_CONTROL_VALUE));
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_name_carries_a_hash_of_the_text() {
        let name = stylesheet_name();
        assert!(
            name.starts_with("style.") && name.ends_with(".css"),
            "{name}"
        );
        assert_eq!(name.len(), "style.".len() + 16 + ".css".len());
        let response = stylesheet_response();
        assert_eq!(response.headers()[CONTENT_TYPE], "text/css; charset=utf-8");
        assert_eq!(response.headers()[CACHE_CONTROL], CACHE_CONTROL_VALUE);
    }

    #[test]
    fn the_stylesheet_sets_no_remote_resource() {
        // The pages' policy is `default-src 'self'`: a font or image from
        // elsewhere would be refused, so the sheet names none.
        assert!(!STYLESHEET.contains("url("));
        assert!(!STYLESHEET.contains("@import"));
    }
}
