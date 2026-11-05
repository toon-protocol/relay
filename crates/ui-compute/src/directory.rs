//! The Provider Directory as this relay's store holds it: each Listing
//! (kind 30432) joined to the Provider Profile (kind 10432) its `a` tag
//! names. Pure reading of events; nothing here touches the store.

use std::collections::HashMap;

use nostr::event::{Event, Kind};
use nostr::key::PublicKey;
use relay_ui::chrome::param;
use relay_ui::meta::summary;
use serde_json::Value;

/// A Provider Profile.
pub(crate) const PROFILE_KIND: u16 = 10_432;
/// A Listing.
pub(crate) const LISTING_KIND: u16 = 30_432;
/// A Liveness.
pub(crate) const LIVENESS_KIND: u16 = 10_433;
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
    /// The Listing's every `l` tag as `name:value`, lowercased: isolation,
    /// arch, hidden, gpu.
    labels: Vec<String>,
    /// Every `t` tag, lowercased: the capabilities.
    caps: Vec<String>,
    /// Whether the Listing or its Profile is tagged `l` `hidden:true`: an
    /// Anyone-only provider, whose host is never shown.
    pub(crate) hidden: bool,
    /// The Listing's `d` tag: the key of its capacity in a Liveness.
    d: String,
    /// µUSDC per Lease Interval.
    pub(crate) price: Option<u64>,
    /// µUSDC per Lease Interval for a Warm Standby; `None` sells no standbys.
    pub(crate) standby_price: Option<u64>,
    pub(crate) lease_interval_s: Option<u64>,
    /// Whether the provider holds an unexpired Liveness.
    pub(crate) live: bool,
    /// What the provider's Liveness says is available for this Listing.
    pub(crate) available: Option<u64>,
    created_at: u64,
}

impl Listing {
    /// The price per hour, when the Listing states a price and an interval.
    pub(crate) fn per_hour(&self) -> Option<PerHour> {
        price_per_hour(self.price?, self.lease_interval_s?)
    }
}

/// A price per hour, exactly: `whole + remainder / interval` µUSDC, the
/// fraction in lowest terms so equal prices are equal values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct PerHour {
    pub(crate) whole: u128,
    pub(crate) remainder: u64,
    pub(crate) interval: u64,
}

impl Ord for PerHour {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Both remainders are below their interval, so both products fit.
        self.whole.cmp(&other.whole).then_with(|| {
            (u128::from(self.remainder) * u128::from(other.interval))
                .cmp(&(u128::from(other.remainder) * u128::from(self.interval)))
        })
    }
}

impl PartialOrd for PerHour {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// `price × 3600 / interval_s` in integer arithmetic, with nothing rounded
/// away. `None` for a zero interval. A pure function.
pub(crate) fn price_per_hour(price: u64, interval_s: u64) -> Option<PerHour> {
    if interval_s == 0 {
        return None;
    }
    let total = u128::from(price) * 3600;
    let interval = u128::from(interval_s);
    let remainder = total % interval;
    let common = gcd(remainder, interval);
    Some(PerHour {
        whole: total / interval,
        // Below `interval_s`, so both fit.
        remainder: u64::try_from(remainder / common).ok()?,
        interval: u64::try_from(interval / common).ok()?,
    })
}

fn gcd(mut a: u128, mut b: u128) -> u128 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// Marks each of `listings` live or down from `liveness` (kind 10433
/// events) at `now`, and fills in its capacity. A provider is live while it
/// has a Liveness that carries no expiration, or one still ahead of `now`;
/// its newest Liveness decides, since it replaces the older ones.
pub(crate) fn apply_liveness(listings: &mut [Listing], liveness: &[Event], now: u64) {
    let mut newest: HashMap<PublicKey, &Event> = HashMap::new();
    for event in liveness
        .iter()
        .filter(|e| e.kind == Kind::from(LIVENESS_KIND))
    {
        let later = newest
            .get(&event.pubkey)
            .is_none_or(|had| (had.created_at, had.id) < (event.created_at, event.id));
        if later {
            newest.insert(event.pubkey, event);
        }
    }
    newest.retain(|_, event| expiration(event).is_none_or(|at| at > now));
    for listing in listings {
        let Some(event) = newest.get(&listing.provider) else {
            listing.live = false;
            listing.available = None;
            continue;
        };
        listing.live = true;
        listing.available = json(&event.content)
            .get("available")
            .and_then(|available| available.get(&listing.d))
            .and_then(count);
    }
}

/// Live providers first, then cheapest per hour first; a Listing with no
/// stated price last among its kind. Stable, so equal Listings keep their
/// order.
pub(crate) fn sort_for_buyers(listings: &mut [Listing]) {
    listings.sort_by(|a, b| {
        b.live
            .cmp(&a.live)
            .then_with(|| match (a.per_hour(), b.per_hour()) {
                (Some(x), Some(y)) => x.cmp(&y),
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => std::cmp::Ordering::Equal,
            })
    });
}

fn expiration(event: &Event) -> Option<u64> {
    event.tags.iter().find_map(|tag| match tag.as_slice() {
        [name, at, ..] if name == "expiration" => at.parse().ok(),
        _ => None,
    })
}

/// A whole count written as a JSON number or a string of digits.
fn count(value: &Value) -> Option<u64> {
    match value {
        Value::Number(n) => n.as_u64(),
        Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
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
    let labels = tag_values(listing, "l");
    let hidden = [&labels, &tag_values(profile, "l")]
        .iter()
        .any(|labels| labels.iter().any(|l| l == "hidden:true"));
    Some(Listing {
        provider: listing.pubkey,
        provider_name: text(&json(&profile.content), "name"),
        name: text(&content, "name").unwrap_or_else(|| d.to_owned()),
        resources: resources(content.get("resources")),
        arch,
        hidden,
        labels,
        caps: tag_values(listing, "t"),
        d: d.to_owned(),
        price: content.get("price").and_then(count),
        standby_price: content.get("standby_price").and_then(count),
        lease_interval_s: content.get("lease_interval_s").and_then(count),
        live: false,
        available: None,
        created_at: listing.created_at.as_secs(),
    })
}

/// The first value of each `name` tag, lowercased.
fn tag_values(event: &Event, name: &str) -> Vec<String> {
    event
        .tags
        .iter()
        .filter_map(|tag| match tag.as_slice() {
            [tag_name, value, ..] if tag_name == name => Some(value.trim().to_lowercase()),
            _ => None,
        })
        .collect()
}

/// What the directory is narrowed to. An empty field is no narrowing; each
/// field a Listing must match, case aside.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct Filters {
    pub(crate) arch: String,
    pub(crate) isolation: String,
    pub(crate) gpu: String,
    pub(crate) cap: String,
    /// `Some(true)` keeps only Hidden Providers, `Some(false)` only the rest.
    pub(crate) hidden: Option<bool>,
}

impl Filters {
    /// From the raw query string; unknown names and a `hidden` other than
    /// `true` or `false` are ignored.
    pub(crate) fn from_query(query: Option<&str>) -> Self {
        let get = |name: &str| {
            param(query, name)
                .map(|v| v.trim().to_owned())
                .unwrap_or_default()
        };
        Self {
            arch: get("arch"),
            isolation: get("isolation"),
            gpu: get("gpu"),
            cap: get("cap"),
            hidden: match get("hidden").to_lowercase().as_str() {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            },
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Whether `listing` passes every filter.
    pub(crate) fn matches(&self, listing: &Listing) -> bool {
        let label = |name: &str, want: &str| {
            want.is_empty() || {
                let want = format!("{name}:{}", want.to_lowercase());
                listing.labels.contains(&want)
            }
        };
        label("arch", &self.arch)
            && label("isolation", &self.isolation)
            && label("gpu", &self.gpu)
            && (self.cap.is_empty() || listing.caps.contains(&self.cap.to_lowercase()))
            && self.hidden.is_none_or(|hidden| hidden == listing.hidden)
    }
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
    use nostr::types::Timestamp;

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

    fn priced(provider: &Keys, d: &str, price: u64, interval: u64) -> Event {
        listing(
            provider,
            d,
            &format!(r#"{{"price":{price},"lease_interval_s":{interval}}}"#),
            provider,
        )
    }

    fn liveness(keys: &Keys, content: &str, expiration: u64) -> Event {
        signed(
            keys,
            LIVENESS_KIND,
            content,
            &[tag(&["expiration", &expiration.to_string()])],
        )
    }

    #[test]
    fn price_per_hour_is_exact_integer_arithmetic() {
        let per = |price, interval| price_per_hour(price, interval).unwrap();
        assert_eq!(
            per(10, 3600),
            PerHour {
                whole: 10,
                remainder: 0,
                interval: 1
            }
        );
        assert_eq!(
            per(5, 60),
            PerHour {
                whole: 300,
                remainder: 0,
                interval: 1
            }
        );
        assert_eq!(
            per(1, 7200),
            PerHour {
                whole: 0,
                remainder: 1,
                interval: 2
            }
        );
        // The same price per hour from a different interval is equal.
        assert_eq!(per(1, 7200), per(2, 14_400));
        // 1000 × 3600 / 7 = 514285 + 5/7: nothing rounded away.
        assert_eq!(
            per(1000, 7),
            PerHour {
                whole: 514_285,
                remainder: 5,
                interval: 7
            }
        );
        assert_eq!(price_per_hour(1, 0), None);
        let max = per(u64::MAX, 1);
        assert_eq!(max.whole, u128::from(u64::MAX) * 3600);
        assert!(per(1, 3) < per(1, 2));
        assert!(per(1, 1800) < per(7, 7200));
    }

    #[test]
    fn standby_price_is_read_and_absence_means_no_standbys() {
        let provider = Keys::generate();
        let profile = signed(&provider, PROFILE_KIND, "{}", &[]);
        let with = listing(
            &provider,
            "a",
            r#"{"price":"9","standby_price":4}"#,
            &provider,
        );
        let without = priced(&provider, "b", 9, 60);
        let joined = join(&[profile], &[with, without]);
        let by = |d: &str| joined.iter().find(|l| l.d == d).unwrap();
        assert_eq!(by("a").standby_price, Some(4));
        assert_eq!(by("a").price, Some(9));
        assert_eq!(by("b").standby_price, None);
    }

    #[test]
    fn live_means_an_unexpired_liveness_and_carries_capacity() {
        let (up, lapsed, silent) = (Keys::generate(), Keys::generate(), Keys::generate());
        let profiles: Vec<Event> = [&up, &lapsed, &silent]
            .iter()
            .map(|k| signed(k, PROFILE_KIND, "{}", &[]))
            .collect();
        let listings = [
            priced(&up, "s", 1, 60),
            priced(&lapsed, "s", 1, 60),
            priced(&silent, "s", 1, 60),
        ];
        let mut joined = join(&profiles, &listings);
        let events = [
            liveness(&up, r#"{"available":{"s":3,"other":9}}"#, 2_000),
            liveness(&lapsed, r#"{"available":{"s":5}}"#, 1_000),
        ];
        apply_liveness(&mut joined, &events, 1_500);
        for listing in &joined {
            if listing.provider == up.public_key() {
                assert!(listing.live);
                assert_eq!(listing.available, Some(3));
            } else {
                assert!(!listing.live);
                assert_eq!(listing.available, None);
            }
        }
    }

    #[test]
    fn an_expired_newest_liveness_is_down_though_an_older_one_is_not() {
        let provider = Keys::generate();
        let profile = signed(&provider, PROFILE_KIND, "{}", &[]);
        let mut joined = join(&[profile], &[priced(&provider, "s", 1, 60)]);
        let at = |created: u64, expires: u64| {
            EventBuilder::new(Kind::from(LIVENESS_KIND), "{}")
                .tags([Tag::parse(["expiration", &expires.to_string()]).unwrap()])
                .custom_created_at(Timestamp::from(created))
                .finalize(&provider)
                .unwrap()
        };
        apply_liveness(&mut joined, &[at(100, 2_000), at(200, 1_000)], 1_500);
        assert!(!joined[0].live);
    }

    #[test]
    fn live_sorts_above_down_then_cheapest_per_hour() {
        let (up, down) = (Keys::generate(), Keys::generate());
        let profiles = [
            signed(&up, PROFILE_KIND, "{}", &[]),
            signed(&down, PROFILE_KIND, "{}", &[]),
        ];
        let listings = [
            priced(&down, "cheap-down", 1, 3600),
            priced(&up, "dear-up", 100, 3600),
            // 60 per minute is 3600 per hour; 50 per hour is cheaper.
            priced(&up, "per-minute", 60, 60),
            priced(&up, "cheap-up", 50, 3600),
        ];
        let mut joined = join(&profiles, &listings);
        apply_liveness(&mut joined, &[liveness(&up, "{}", 9_999)], 1);
        sort_for_buyers(&mut joined);
        let names: Vec<&str> = joined.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["cheap-up", "dear-up", "per-minute", "cheap-down"]);
    }

    #[test]
    fn filters_match_the_l_and_t_tags_alone_and_combined() {
        let provider = Keys::generate();
        let profile = signed(
            &provider,
            PROFILE_KIND,
            "{}",
            &[tag(&["l", "hidden:true"]), tag(&["l", "arch:riscv64"])],
        );
        let a = format!("10432:{}:", provider.public_key().to_hex());
        let with = |d: &str, extra: &[&[&str]]| {
            let mut tags = vec![tag(&["d", d]), tag(&["a", &a])];
            tags.extend(extra.iter().map(|t| tag(t)));
            signed(&provider, LISTING_KIND, "{}", &tags)
        };
        let other = Keys::generate();
        let other_profile = signed(&other, PROFILE_KIND, "{}", &[]);
        let other_listing = signed(
            &other,
            LISTING_KIND,
            "{}",
            &[
                tag(&["d", "plain"]),
                tag(&["a", &format!("10432:{}:", other.public_key().to_hex())]),
            ],
        );
        let joined = join(
            &[profile, other_profile],
            &[
                with(
                    "gpu-vm",
                    &[
                        &["l", "arch:arm64"],
                        &["l", "isolation:vm"],
                        &["l", "gpu:a100"],
                        &["t", "Docker"],
                    ],
                ),
                with(
                    "cpu",
                    &[&["l", "arch:x86_64"], &["l", "isolation:container"]],
                ),
                other_listing,
            ],
        );
        let names = |f: Filters| -> Vec<String> {
            let mut n: Vec<String> = joined
                .iter()
                .filter(|l| f.matches(l))
                .map(|l| l.name.clone())
                .collect();
            n.sort();
            n
        };
        let f = |arch: &str, isolation: &str, gpu: &str, cap: &str, hidden: Option<bool>| Filters {
            arch: arch.into(),
            isolation: isolation.into(),
            gpu: gpu.into(),
            cap: cap.into(),
            hidden,
        };
        assert_eq!(names(Filters::default()), ["cpu", "gpu-vm", "plain"]);
        assert_eq!(names(f("ARM64", "", "", "", None)), ["gpu-vm"]);
        assert_eq!(names(f("", "container", "", "", None)), ["cpu"]);
        assert_eq!(names(f("", "", "a100", "", None)), ["gpu-vm"]);
        assert_eq!(names(f("", "", "", "docker", None)), ["gpu-vm"]);
        assert_eq!(names(f("", "", "", "", Some(true))), ["cpu", "gpu-vm"]);
        assert_eq!(names(f("", "", "", "", Some(false))), ["plain"]);
        assert_eq!(
            names(f("arm64", "vm", "a100", "docker", Some(true))),
            ["gpu-vm"]
        );
        assert!(names(f("arm64", "container", "", "", None)).is_empty());
        assert!(names(f("riscv64", "", "", "", None)).is_empty());
    }
}
