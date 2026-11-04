//! The Provider Directory as this relay's store holds it: each Listing
//! (kind 30432) joined to the Provider Profile (kind 10432) its `a` tag
//! names. Pure reading of events; nothing here touches the store.

use std::collections::HashMap;

use nostr::event::{Event, Kind};
use nostr::key::PublicKey;
use relay_ui::meta::summary;
use serde_json::Value;

/// A Provider Profile.
pub(crate) const PROFILE_KIND: u16 = 10_432;
/// A Listing.
pub(crate) const LISTING_KIND: u16 = 30_432;
/// The `L` namespace every TOON Compute event carries.
pub(crate) const LABEL_NAMESPACE: &str = "toon.network";

/// The longest a resource or an architecture is drawn.
const FIELD_CHARS: usize = 80;

/// One Listing, joined to its provider.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Listing {
    pub(crate) provider: PublicKey,
    /// The provider's own name for itself, when its Profile gives one.
    pub(crate) provider_name: Option<String>,
    /// The Listing's name: its content's `name`, else its `d` tag.
    pub(crate) name: String,
    /// `what: amount` for each resource the Listing sells.
    pub(crate) resources: Vec<(String, String)>,
    pub(crate) arch: Option<String>,
    created_at: u64,
}

/// The Listings among `listings` whose provider's Profile is among
/// `profiles`, newest first within a provider, providers in key order. A
/// Listing with no Profile is left out: the spec makes it unpurchasable. A
/// pure function.
pub(crate) fn join(profiles: &[Event], listings: &[Event]) -> Vec<Listing> {
    let mut held: HashMap<PublicKey, &Event> = HashMap::new();
    for profile in profiles
        .iter()
        .filter(|e| e.kind == Kind::from(PROFILE_KIND))
    {
        let newest = held
            .get(&profile.pubkey)
            .is_none_or(|had| (had.created_at, had.id) < (profile.created_at, profile.id));
        if newest {
            held.insert(profile.pubkey, profile);
        }
    }
    let mut out: Vec<Listing> = listings
        .iter()
        .filter(|e| e.kind == Kind::from(LISTING_KIND))
        .filter_map(|listing| {
            let profile = held.get(&profile_of(listing)?)?;
            read(listing, profile)
        })
        .collect();
    out.sort_by(|a, b| {
        a.provider
            .cmp(&b.provider)
            .then_with(|| b.created_at.cmp(&a.created_at))
            .then_with(|| a.name.cmp(&b.name))
    });
    out
}

/// The provider a Listing's `a` tag (`10432:<pubkey>:`) names, if it is the
/// Listing's own signer: a Listing cannot borrow another provider's Profile.
fn profile_of(listing: &Event) -> Option<PublicKey> {
    listing.tags.iter().find_map(|tag| {
        let [name, coordinate, ..] = tag.as_slice() else {
            return None;
        };
        if name != "a" {
            return None;
        }
        let mut parts = coordinate.split(':');
        let (kind, key, d) = (parts.next()?, parts.next()?, parts.next()?);
        if kind.parse() != Ok(PROFILE_KIND) || !d.is_empty() || parts.next().is_some() {
            return None;
        }
        let key = PublicKey::from_hex(key).ok()?;
        (key == listing.pubkey).then_some(key)
    })
}

fn read(listing: &Event, profile: &Event) -> Option<Listing> {
    let content = json(&listing.content);
    let d = listing.tags.iter().find_map(|tag| match tag.as_slice() {
        [name, value, ..] if name == "d" => Some(value.as_str()),
        _ => None,
    })?;
    let text = |value: &Value, field: &str| {
        value
            .get(field)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
            .map(str::to_owned)
    };
    let arch = text(&content, "arch")
        .or_else(|| label(listing, "arch"))
        .map(|arch| summary(&arch, FIELD_CHARS));
    Some(Listing {
        provider: listing.pubkey,
        provider_name: text(&json(&profile.content), "name"),
        name: text(&content, "name").unwrap_or_else(|| d.to_owned()),
        resources: resources(content.get("resources")),
        arch,
        created_at: listing.created_at.as_secs(),
    })
}

/// The value of an `l` tag written `<name>:<value>`.
fn label(event: &Event, name: &str) -> Option<String> {
    event.tags.iter().find_map(|tag| match tag.as_slice() {
        [tag_name, value, ..] if tag_name == "l" => value
            .strip_prefix(name)?
            .strip_prefix(':')
            .map(str::to_owned),
        _ => None,
    })
}

fn json(content: &str) -> Value {
    serde_json::from_str(content).unwrap_or(Value::Null)
}

/// What a Listing's `resources` says, as `(what, amount)` pairs: an
/// object's fields in name order, or a lone text.
fn resources(value: Option<&Value>) -> Vec<(String, String)> {
    let shown = |value: &Value| match value {
        Value::String(text) => summary(text, FIELD_CHARS),
        other => summary(&other.to_string(), FIELD_CHARS),
    };
    match value {
        Some(Value::Object(fields)) => fields
            .iter()
            .map(|(what, amount)| (summary(what, FIELD_CHARS), shown(amount)))
            .collect(),
        Some(Value::Null) | None => Vec::new(),
        Some(other) => vec![(String::new(), shown(other))],
    }
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;

    use super::*;

    fn signed(keys: &Keys, kind: u16, content: &str, tags: &[Vec<String>]) -> Event {
        EventBuilder::new(Kind::from(kind), content)
            .tags(tags.iter().map(|tag| Tag::parse(tag.clone()).unwrap()))
            .finalize(keys)
            .unwrap()
    }

    fn tag(parts: &[&str]) -> Vec<String> {
        parts.iter().map(|part| (*part).to_owned()).collect()
    }

    fn listing(keys: &Keys, d: &str, content: &str, a_key: &Keys) -> Event {
        signed(
            keys,
            LISTING_KIND,
            content,
            &[
                tag(&["d", d]),
                tag(&["a", &format!("10432:{}:", a_key.public_key().to_hex())]),
            ],
        )
    }

    #[test]
    fn a_listing_is_shown_only_with_its_own_providers_profile() {
        let (provider, orphan) = (Keys::generate(), Keys::generate());
        let profile = signed(&provider, PROFILE_KIND, r#"{"name":"Tidal"}"#, &[]);
        let listings = [
            listing(&provider, "small", r#"{"arch":"arm64"}"#, &provider),
            listing(&orphan, "lost", "{}", &orphan),
            // Naming another provider's Profile is not a join.
            listing(&orphan, "borrowed", "{}", &provider),
        ];
        let joined = join(&[profile], &listings);
        assert_eq!(joined.len(), 1);
        assert_eq!(joined[0].name, "small");
        assert_eq!(joined[0].provider_name.as_deref(), Some("Tidal"));
        assert_eq!(joined[0].arch.as_deref(), Some("arm64"));
    }

    #[test]
    fn the_a_tag_names_a_profile_by_kind_and_key_alone() {
        let provider = Keys::generate();
        let key = provider.public_key().to_hex();
        let profile = signed(&provider, PROFILE_KIND, "{}", &[]);
        let with_a = |a: &str| {
            signed(
                &provider,
                LISTING_KIND,
                "{}",
                &[tag(&["d", a]), tag(&["a", a])],
            )
        };
        let listings = [
            with_a(&format!("10432:{key}:")),
            with_a(&format!("10432:{key}:extra")),
            with_a(&format!("10432:{key}")),
            with_a(&format!("30432:{key}:")),
        ];
        let joined = join(&[profile], &listings);
        assert_eq!(joined.len(), 1);
        assert_eq!(joined[0].name, format!("10432:{key}:"));
    }

    #[test]
    fn resources_and_arch_are_read_from_the_content_or_the_labels() {
        let provider = Keys::generate();
        let profile = signed(&provider, PROFILE_KIND, "{}", &[]);
        let mut event = listing(
            &provider,
            "d",
            r#"{"name":"Small","resources":{"vcpu":2,"memory":"4 GiB"}}"#,
            &provider,
        );
        let joined = join(std::slice::from_ref(&profile), std::slice::from_ref(&event));
        assert_eq!(joined[0].name, "Small");
        assert_eq!(
            joined[0].resources,
            vec![
                ("memory".to_owned(), "4 GiB".to_owned()),
                ("vcpu".to_owned(), "2".to_owned())
            ]
        );
        assert_eq!(joined[0].arch, None);
        event = signed(
            &provider,
            LISTING_KIND,
            "not json",
            &[
                tag(&["d", "x"]),
                tag(&["a", &format!("10432:{}:", provider.public_key().to_hex())]),
                tag(&["l", "arch:x86_64"]),
            ],
        );
        let joined = join(&[profile], &[event]);
        assert_eq!(joined[0].name, "x");
        assert_eq!(joined[0].arch.as_deref(), Some("x86_64"));
    }
}
