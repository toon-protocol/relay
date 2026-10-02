//! How a subscriber proves which key it holds (#215): NIP-98 on the HTTP
//! surfaces and NIP-42 on the WebSocket.
//!
//! Both are an event the subscriber signs, so a proof is whatever
//! [`VerifiedEvent::verify`] lets through plus the checks the two NIPs add,
//! and the key is the event's `pubkey`. Neither names a payer: a balance
//! belongs to the key that signed, not to whoever paid.
//!
//! The relay cannot see which URL a client used when it is a hidden service
//! or sits behind a reverse proxy, so both checks accept a URL whose host is
//! the relay's own, as NIP-42 already allows for `relay`. The relay's host is
//! the one in `TOON_RELAY_URL`.

use axum::http::HeaderMap;
use axum::http::header::AUTHORIZATION;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use hyper::Uri;
use nostr::event::{Event, Kind};
use nostr::key::PublicKey;
use sha2::{Digest, Sha256};

use crate::VerifiedEvent;

/// NIP-98's HTTP authorization event.
const HTTP_AUTH: u16 = 27235;

/// How far from the relay's clock a NIP-98 event's `created_at` may be, in
/// seconds (the draft: 60).
const HTTP_AUTH_WINDOW: u64 = 60;

/// How far from the relay's clock a NIP-42 event's `created_at` may be, in
/// seconds (NIP-42: about ten minutes).
const CLIENT_AUTH_WINDOW: u64 = 600;

/// The scheme of the `Authorization` header NIP-98 defines.
const SCHEME: &str = "Nostr ";

/// The host this relay is reached at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Hosts {
    own: (String, u16),
}

impl Hosts {
    /// The host of `relay_url`, which configuration has already shown to be a
    /// `ws`, `wss`, `http` or `https` URL with an authority.
    pub(crate) fn of(relay_url: &str) -> Option<Self> {
        host_of(relay_url).map(|own| Self { own })
    }

    /// Whether `url` names this relay's host.
    fn name(&self, url: &str) -> bool {
        host_of(url).is_some_and(|host| host == self.own)
    }
}

/// A URL's host in lower case, and its port: the one stated, or its
/// scheme's own.
fn host_of(url: &str) -> Option<(String, u16)> {
    let uri = url.parse::<Uri>().ok()?;
    let port = match (uri.port_u16(), uri.scheme_str()) {
        (Some(port), _) => port,
        (None, Some("ws" | "http")) => 80,
        (None, Some("wss" | "https")) => 443,
        _ => return None,
    };
    Some((uri.host()?.to_ascii_lowercase(), port))
}

/// The value of the first tag named `name`, if it has one.
fn tag<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    event
        .tags
        .iter()
        .find(|tag| tag.as_slice().first().map(String::as_str) == Some(name))
        .and_then(|tag| tag.as_slice().get(1))
        .map(String::as_str)
}

fn within(event: &Event, now: u64, window: u64) -> bool {
    event.created_at.as_secs().abs_diff(now) <= window
}

/// The lower-case hex SHA-256 of `body`: what a NIP-98 `payload` tag holds.
fn sha256_hex(body: &[u8]) -> String {
    Sha256::digest(body)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// The key that signed the NIP-98 authorization in `headers` for a `method`
/// request, at `now`. `body` is the request's body when it has one, and the
/// `payload` tag must then be its hash; a request with no body states none.
/// The error says what was wrong, for a person.
pub(crate) fn http_authorization(
    headers: &HeaderMap,
    method: &str,
    body: Option<&[u8]>,
    hosts: &Hosts,
    now: u64,
) -> Result<PublicKey, &'static str> {
    let header = headers
        .get(AUTHORIZATION)
        .ok_or("an Authorization header is required")?
        .to_str()
        .map_err(|_| "the Authorization header is not text")?;
    let token = header
        .get(..SCHEME.len())
        .filter(|scheme| scheme.eq_ignore_ascii_case(SCHEME))
        .map(|_| &header[SCHEME.len()..])
        .ok_or("the Authorization scheme must be Nostr")?;
    let json = STANDARD
        .decode(token.trim())
        .map_err(|_| "the authorization is not base64")?;
    let event = serde_json::from_slice::<Event>(&json)
        .map_err(|_| "the authorization is not a Nostr event")
        .and_then(|event| {
            VerifiedEvent::verify(event)
                .map_err(|_| "the authorization's id or signature does not verify")
        })?;
    let event = event.event();
    if event.kind != Kind::from(HTTP_AUTH) {
        return Err("the authorization is not a kind 27235 event");
    }
    if !tag(event, "method").is_some_and(|stated| stated.eq_ignore_ascii_case(method)) {
        return Err("the authorization's method tag is not the request's method");
    }
    if !tag(event, "u").is_some_and(|url| hosts.name(url)) {
        return Err("the authorization's u tag does not name this relay");
    }
    if body.is_some_and(|body| tag(event, "payload") != Some(sha256_hex(body).as_str())) {
        return Err("the authorization's payload tag is not the hash of the body");
    }
    if !within(event, now, HTTP_AUTH_WINDOW) {
        return Err(
            "the authorization's created_at is more than 60 seconds from the relay's clock",
        );
    }
    Ok(event.pubkey)
}

/// The key that signed `event`, a NIP-42 `AUTH` answering `challenge` at
/// `now`. The error says what was wrong, for a person.
pub(crate) fn client_authentication(
    event: Event,
    challenge: &str,
    hosts: &Hosts,
    now: u64,
) -> Result<PublicKey, &'static str> {
    if event.kind != Kind::Authentication {
        return Err("invalid authentication event kind");
    }
    let verified =
        VerifiedEvent::verify(event).map_err(|_| "the event's id or signature does not verify")?;
    let event = verified.event();
    if tag(event, "challenge") != Some(challenge) {
        return Err("received invalid challenge");
    }
    if !tag(event, "relay").is_some_and(|url| hosts.name(url)) {
        return Err("the relay tag does not name this relay");
    }
    if !within(event, now, CLIENT_AUTH_WINDOW) {
        return Err("created_at is too far from the relay's clock");
    }
    Ok(event.pubkey)
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;
    use nostr::types::Timestamp;

    use super::*;

    const NOW: u64 = 1_790_000_000;

    fn hosts() -> Hosts {
        Hosts::of("wss://Relay.Example").expect("a URL")
    }

    fn signed(keys: &Keys, kind: u16, at: u64, tags: &[&[&str]]) -> Event {
        let tags = tags
            .iter()
            .map(|tag| Tag::parse(tag.iter().copied()).expect("a tag"));
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags)
            .custom_created_at(Timestamp::from(at))
            .finalize(keys)
            .expect("signed")
    }

    fn headers(event: &Event) -> HeaderMap {
        let mut headers = HeaderMap::new();
        let token = STANDARD.encode(serde_json::to_vec(event).expect("JSON"));
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_str(&format!("Nostr {token}")).expect("a header"),
        );
        headers
    }

    fn post(keys: &Keys, body: &[u8], overrides: &[(&str, &str)]) -> Event {
        let payload = sha256_hex(body);
        let mut tags = vec![
            ("u", "https://relay.example/"),
            ("method", "POST"),
            ("payload", payload.as_str()),
        ];
        for (name, value) in overrides {
            tags.retain(|(have, _)| have != name);
            tags.push((name, value));
        }
        let tags: Vec<Vec<&str>> = tags.iter().map(|(a, b)| vec![*a, *b]).collect();
        let tags: Vec<&[&str]> = tags.iter().map(Vec::as_slice).collect();
        signed(keys, HTTP_AUTH, NOW, &tags)
    }

    #[test]
    fn a_hash_of_the_body_hexes_as_sha256() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn a_host_is_the_relays_whatever_its_scheme_case_path_or_default_port() {
        let hosts = hosts();
        for named in [
            "wss://relay.example",
            "https://relay.example/",
            "https://RELAY.example:443/some/path",
            "wss://relay.example/",
        ] {
            assert!(hosts.name(named), "{named}");
        }
        for other in [
            "wss://relay.example:8443",
            "https://elsewhere.example/",
            "http://relay.example/",
            "relay.example",
        ] {
            assert!(!hosts.name(other), "{other}");
        }
    }

    #[test]
    fn an_authorization_that_holds_names_the_key_that_signed_it() {
        let keys = Keys::generate();
        let body = br#"{"filter":{}}"#;
        let event = post(&keys, body, &[]);
        assert_eq!(
            http_authorization(&headers(&event), "POST", Some(body), &hosts(), NOW),
            Ok(keys.public_key())
        );
    }

    #[test]
    fn a_get_states_no_payload_and_a_body_less_request_is_not_asked_for_one() {
        let keys = Keys::generate();
        let event = signed(
            &keys,
            HTTP_AUTH,
            NOW,
            &[&["u", "https://relay.example/"], &["method", "GET"]],
        );
        assert_eq!(
            http_authorization(&headers(&event), "GET", None, &hosts(), NOW),
            Ok(keys.public_key())
        );
    }

    #[test]
    fn an_authorization_that_does_not_hold_is_refused_for_the_reason() {
        let keys = Keys::generate();
        let body = br#"{"filter":{}}"#;
        let refused = |event: Event, method: &str, body: Option<&[u8]>, now: u64| {
            http_authorization(&headers(&event), method, body, &hosts(), now)
                .expect_err("it is refused")
        };

        assert!(
            http_authorization(&HeaderMap::new(), "POST", Some(body), &hosts(), NOW)
                .expect_err("nothing")
                .contains("required")
        );
        assert!(
            refused(
                post(&keys, body, &[("method", "GET")]),
                "POST",
                Some(body),
                NOW
            )
            .contains("method")
        );
        assert!(
            refused(
                post(&keys, body, &[("u", "https://elsewhere.example/")]),
                "POST",
                Some(body),
                NOW
            )
            .contains("u tag")
        );
        assert!(refused(post(&keys, b"other", &[]), "POST", Some(body), NOW).contains("payload"));
        assert!(
            refused(post(&keys, body, &[]), "POST", Some(body), NOW + 61).contains("60 seconds")
        );
        assert!(
            refused(post(&keys, body, &[]), "POST", Some(body), NOW - 61).contains("60 seconds")
        );
        let wrong_kind = signed(&keys, 1, NOW, &[]);
        assert!(refused(wrong_kind, "POST", Some(body), NOW).contains("27235"));

        let mut bad = headers(&post(&keys, body, &[]));
        bad.insert(AUTHORIZATION, HeaderValue::from_static("Bearer x"));
        assert!(
            http_authorization(&bad, "POST", Some(body), &hosts(), NOW)
                .expect_err("not Nostr")
                .contains("scheme")
        );
        bad.insert(AUTHORIZATION, HeaderValue::from_static("Nostr !!!"));
        assert!(
            http_authorization(&bad, "POST", Some(body), &hosts(), NOW)
                .expect_err("not base64")
                .contains("base64")
        );
    }

    #[test]
    fn an_authorization_is_not_refused_for_having_been_seen_before() {
        let keys = Keys::generate();
        let body = b"{}";
        let event = post(&keys, body, &[]);
        for _ in 0..2 {
            assert!(
                http_authorization(&headers(&event), "POST", Some(body), &hosts(), NOW).is_ok()
            );
        }
    }

    #[test]
    fn an_auth_event_answers_this_connections_challenge_for_this_relay() {
        let keys = Keys::generate();
        let auth = |challenge: &str, relay: &str, at: u64| {
            signed(
                &keys,
                22242,
                at,
                &[&["relay", relay], &["challenge", challenge]],
            )
        };
        let ok = auth("abc", "wss://relay.example", NOW);
        assert_eq!(
            client_authentication(ok, "abc", &hosts(), NOW),
            Ok(keys.public_key())
        );
        let refused =
            |event: Event| client_authentication(event, "abc", &hosts(), NOW).expect_err("refused");
        assert!(refused(auth("xyz", "wss://relay.example", NOW)).contains("challenge"));
        assert!(refused(auth("abc", "wss://elsewhere.example", NOW)).contains("relay"));
        assert!(refused(auth("abc", "wss://relay.example", NOW - 601)).contains("created_at"));
        assert!(refused(signed(&keys, 1, NOW, &[])).contains("kind"));
    }
}
