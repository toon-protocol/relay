//! Whose eyes a page reads with, without a login: `?as=<npub>` in the URL
//! (ADR 0001 gives the UIs no session). A page that takes it narrows what
//! it shows to that key's view: the people it follows vouch for an app,
//! their reports raise a banner, its mute list hides posts ([`crate::mute`]).
//! Nothing is kept: the key is in the link, and anyone can read as anyone,
//! since every list read here is public.

use nostr::event::{Event, Kind};
use nostr::filter::Filter;
use nostr::key::PublicKey;
use relay_ui::ReadOnlyStore;
use relay_ui::chrome::{escape, npub};

use crate::kinds::FOLLOWS;
use crate::parse_pubkey;

/// The value of `name` in a query string, percent-decoded (`+` is a space).
/// A pure function.
pub(crate) fn param(query: Option<&str>, name: &str) -> Option<String> {
    let raw = query?.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == name).then_some(value)
    })?;
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut at = 0;
    while at < bytes.len() {
        match bytes[at] {
            b'+' => out.push(b' '),
            b'%' => {
                let hex = |offset: usize| {
                    bytes
                        .get(at + offset)
                        .and_then(|b| char::from(*b).to_digit(16))
                };
                match (hex(1), hex(2)) {
                    (Some(high), Some(low)) => {
                        out.push(u8::try_from(high * 16 + low).unwrap_or(b'?'));
                        at += 2;
                    }
                    _ => out.push(b'%'),
                }
            }
            byte => out.push(byte),
        }
        at += 1;
    }
    String::from_utf8(out).ok()
}

/// The key `?as=` names, if it names one.
pub(crate) fn as_key(query: Option<&str>) -> Option<PublicKey> {
    param(query, "as").and_then(|value| parse_pubkey(&value))
}

/// `?as=<npub>` for a link that keeps the lens, or nothing.
pub(crate) fn as_query(viewer: Option<&PublicKey>, first: bool) -> String {
    match viewer {
        Some(key) => format!("{}as={}", if first { '?' } else { '&' }, npub(key)),
        None => String::new(),
    }
}

/// The keys `viewer`'s newest kind 3 follow list names, once each.
pub(crate) async fn follows(store: &ReadOnlyStore, viewer: &PublicKey) -> Vec<PublicKey> {
    let lists: Vec<Event> = store
        .query(Filter::new().kind(Kind::from(FOLLOWS)).author(*viewer))
        .await;
    crate::feed::listed_keys(ui_nip34::repo::newest(
        lists.iter().filter(|list| list.pubkey == *viewer),
    ))
}

/// A small form that reloads the page as another key (a `GET` back to the
/// same page, so nothing is written).
pub(crate) fn as_form(viewer: Option<&PublicKey>, label: &str) -> String {
    format!(
        "<form class=\"viewer viewer-inline\" method=\"get\">\
         <label for=\"as-npub\">{}</label>\
         <input class=\"input\" id=\"as-npub\" name=\"as\" placeholder=\"npub1…\" value=\"{}\" autocomplete=\"off\">\
         <button class=\"btn\" type=\"submit\">Read as</button></form>",
        escape(label),
        viewer
            .map(npub)
            .map(|npub| escape(&npub))
            .unwrap_or_default(),
    )
}

#[cfg(test)]
mod tests {
    use nostr::key::Keys;

    use super::*;

    #[test]
    fn a_query_parameter_is_found_and_decoded() {
        assert_eq!(param(Some("a=1&as=x%3Ay"), "as").as_deref(), Some("x:y"));
        assert_eq!(
            param(Some("t=rust+wasm"), "t").as_deref(),
            Some("rust wasm")
        );
        assert_eq!(param(Some("t=%zz"), "t").as_deref(), Some("%zz"));
        assert_eq!(param(Some("t=%4"), "t").as_deref(), Some("%4"));
        assert_eq!(param(Some("x"), "x").as_deref(), Some(""));
        assert_eq!(param(Some("ask=1"), "as"), None);
        assert_eq!(param(None, "as"), None);
    }

    #[test]
    fn the_lens_is_a_key_in_any_spelling() {
        let key = Keys::generate().public_key();
        let query = format!("as=nostr%3A{}", npub(&key));
        assert_eq!(as_key(Some(&query)), Some(key));
        assert_eq!(as_key(Some("as=npub1nope")), None);
        assert_eq!(as_query(Some(&key), true), format!("?as={}", npub(&key)));
        assert_eq!(as_query(None, false), "");
    }
}
