//! Ship posts (proposal 3 of `docs/social-for-builders-ux.md`): what makes
//! an event a launch, and the live demo a launch can link to.
//!
//! A ship post is the maker's voice (rule 2 of §7.0): its author is the
//! owner of a repo it `a`-tags, or a maintainer the owner named (ADR 0002).
//! Two shapes count:
//!
//! - a kind 1 note that `a`-tags such a repo and is marked a launch, by a
//!   `t` tag `ship` or by carrying `imeta` media;
//! - a kind 31990 app event (Shakespeare's) that `a`-tags such a repo and
//!   an NIP-5A site (`35128:…`), which is its live demo.
//!
//! What a launch's live demo is, and how one is found for any repo, is
//! [`crate::demo`]'s. This module holds what a NIP-5A site is and how a
//! gateway addresses it.

use nostr::event::{Event, Kind};
use nostr::key::PublicKey;
use nostr::nips::nip19::{FromBech32, Nip19};
use relay_ui::chrome::npub;
use ui_nip34::repo::{RepoAddress, first_tag};

use crate::kinds::{APP_HANDLER, NOTE, SITE_NAMED, SITE_ROOT};
use crate::media;

/// The repo `event` launches, if it is a ship post: see the module's rules.
/// `is_maker(repo, key)` says whether `key` is an authorized author of
/// `repo`. A pure function.
pub(crate) fn ship_repo(
    event: &Event,
    is_maker: impl Fn(&RepoAddress, &PublicKey) -> bool,
) -> Option<RepoAddress> {
    let marked = match event.kind.as_u16() {
        NOTE => {
            event.tags.iter().map(|tag| tag.as_slice()).any(
                |tag| matches!(tag, [name, value, ..] if name == "t" && value.eq_ignore_ascii_case("ship")),
            ) || !media::imeta(event).is_empty()
        }
        APP_HANDLER => app_site(event).is_some(),
        _ => false,
    };
    if !marked {
        return None;
    }
    RepoAddress::tagged(event, "a")
        .into_iter()
        .find(|repo| is_maker(repo, &event.pubkey))
}

/// A NIP-5A site: a root site (kind 15128) or a named one (kind 35128).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Site {
    author: PublicKey,
    name: Option<String>,
}

impl Site {
    /// The site `event` is, if it is a site manifest.
    pub(crate) fn of(event: &Event) -> Option<Self> {
        match event.kind.as_u16() {
            SITE_ROOT => Some(Self {
                author: event.pubkey,
                name: None,
            }),
            SITE_NAMED => Some(Self {
                author: event.pubkey,
                name: Some(ui_nip34::repo::d_tag(event).to_owned()),
            }),
            _ => None,
        }
    }

    /// The key that signed the site.
    pub(crate) fn author(&self) -> &PublicKey {
        &self.author
    }

    /// The site's address as an `a` tag names it: `35128:<hex>:<d>`, or
    /// `15128:<hex>:` for a root site (NIP-01's form for a replaceable
    /// event).
    pub(crate) fn coordinate(&self) -> String {
        match &self.name {
            Some(name) => format!("{SITE_NAMED}:{}:{name}", self.author.to_hex()),
            None => format!("{SITE_ROOT}:{}:", self.author.to_hex()),
        }
    }

    /// The site an `a` or `A` tag value names: `35128:<hex>:<d>`, or a root
    /// site `15128:<hex>` with or without the trailing colon.
    pub(crate) fn parse(coordinate: &str) -> Option<Self> {
        let mut parts = coordinate.splitn(3, ':');
        let (kind, author) = (parts.next()?, parts.next()?);
        let d = parts.next();
        let author = PublicKey::from_hex(author).ok()?;
        match kind.parse::<u16>().ok()? {
            SITE_NAMED => Some(Self {
                author,
                name: Some(d.filter(|d| !d.is_empty())?.to_owned()),
            }),
            SITE_ROOT if d.is_none_or(str::is_empty) => Some(Self { author, name: None }),
            _ => None,
        }
    }

    /// The site's address on the gateway at `gateway` (an `https` URL), if
    /// its name can be a DNS label: a root site is `<npub>.<host>`, a named
    /// one `<pubkey in base36><name>.<host>`, the canonical formats of
    /// NIP-5A's "Address Formats" (verified against `5A.md` at
    /// nostr-protocol/nips@d47a2ec). The base36 key is the 50 characters the
    /// NIP says it always is, so a key with leading zero digits keeps them.
    pub(crate) fn url(&self, gateway: &str) -> Option<String> {
        let host = gateway
            .strip_prefix("https://")?
            .trim_end_matches('/')
            .split('/')
            .next()
            .filter(|host| !host.is_empty())?;
        let label = match &self.name {
            None => npub(&self.author),
            Some(name) => {
                let valid = (1..=13).contains(&name.len())
                    && !name.ends_with('-')
                    && name
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
                if !valid {
                    return None;
                }
                format!("{}{name}", base36(&self.author.to_bytes()))
            }
        };
        Some(format!("https://{label}.{host}/"))
    }
}

/// The site an app event names in an `a` tag (`35128:<pubkey>:<name>`, or a
/// root site).
pub(crate) fn app_site(event: &Event) -> Option<Site> {
    if event.kind != Kind::from(APP_HANDLER) {
        return None;
    }
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find_map(|tag| match tag {
            [name, value, ..] if name == "a" => Site::parse(value),
            _ => None,
        })
}

/// Who a NIP-34 `nostr://` clone URL names, and which repo of theirs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    /// `nostr://<naddr>` or `nostr://<npub>/[<relay>/]<d>`: an address.
    Repo(RepoAddress),
    /// `nostr://<nip05>/[<relay>/]<d>`: a NIP-05 name (lower case, `_@`
    /// dropped as NIP-05 does) that only its owner's profile can tie to a key.
    Named(String, String),
}

impl Source {
    /// The repo a `source` value names, in any of NIP-34's three clone URL
    /// forms (`34.md`, "Nostr Clone URL format"). A pure function.
    pub(crate) fn parse(source: &str) -> Option<Self> {
        let rest = source.trim().strip_prefix("nostr://")?;
        let parts: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
        match parts.as_slice() {
            [naddr] => RepoAddress::from_naddr(naddr).map(Self::Repo),
            [who, .., last] if parts.len() <= 3 => {
                let d = percent_decode(last);
                match Nip19::from_bech32(who) {
                    Ok(Nip19::Pubkey(key)) => RepoAddress::new(key, &d).map(Self::Repo),
                    Ok(Nip19::Profile(profile)) => {
                        RepoAddress::new(profile.public_key, &d).map(Self::Repo)
                    }
                    Ok(_) => None,
                    Err(_) => {
                        let who = if who.contains('@') {
                            relay_ui::chrome::nip05_identifier(who)?
                        } else {
                            relay_ui::chrome::nip05_identifier(&format!("_@{who}"))?
                        };
                        (!d.is_empty()).then_some(Self::Named(who, d))
                    }
                }
            }
            _ => None,
        }
    }
}

/// Whether a NIP-5A `source` tag names `repo`: a `nostr://` clone URL of it
/// by `naddr`, by the owner's npub, or by the NIP-05 name the owner's
/// profile claims (`nip05`). A pure function.
pub(crate) fn source_names(source: &str, repo: &RepoAddress, nip05: Option<&str>) -> bool {
    match Source::parse(source) {
        Some(Source::Repo(named)) => named == *repo,
        Some(Source::Named(who, d)) => {
            nip05
                .and_then(relay_ui::chrome::nip05_identifier)
                .as_deref()
                == Some(who.as_str())
                && d == repo.d()
        }
        None => false,
    }
}

/// The repo a `nostr://<npub>/[<relay>/]<d>` URL names, when its first part
/// is an npub. A NIP-05 name in that place would need its `.well-known`
/// document fetched to become a key, which the relay never does. A pure
/// function.
pub(crate) fn source_repo(source: &str) -> Option<RepoAddress> {
    let rest = source.trim().strip_prefix("nostr://")?;
    let parts: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
    if parts.len() < 2 {
        return None;
    }
    let owner = crate::parse_pubkey(parts.first()?)?;
    RepoAddress::new(owner, &percent_decode(parts.last()?))
}

/// `%XX` escapes decoded; anything malformed is kept as written.
fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        let decoded = (bytes[index] == b'%')
            .then(|| text.get(index + 1..index + 3))
            .flatten()
            .and_then(|hex| u8::from_str_radix(hex, 16).ok());
        match decoded {
            Some(byte) => {
                out.push(byte);
                index += 3;
            }
            None => {
                out.push(bytes[index]);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `bytes` as a big-endian number in lower-case base 36, padded to the 50
/// digits a 32-byte key takes.
fn base36(bytes: &[u8]) -> String {
    let mut number = bytes.to_vec();
    let mut digits = Vec::new();
    while number.iter().any(|byte| *byte != 0) {
        let mut remainder = 0u32;
        for byte in &mut number {
            let value = (remainder << 8) | u32::from(*byte);
            *byte = u8::try_from(value / 36).unwrap_or(0);
            remainder = value % 36;
        }
        digits.push(b"0123456789abcdefghijklmnopqrstuvwxyz"[remainder as usize]);
    }
    while digits.len() < 50 {
        digits.push(b'0');
    }
    digits.reverse();
    String::from_utf8_lossy(&digits).into_owned()
}

/// What an app event's content says about the app (NIP-89 metadata JSON):
/// its name and its description, when given.
pub(crate) fn app_metadata(event: &Event) -> (Option<String>, Option<String>) {
    let Ok(serde_json::Value::Object(fields)) = serde_json::from_str(&event.content) else {
        return (first_tag(event, "name").map(str::to_owned), None);
    };
    let text = |key: &str| {
        fields
            .get(key)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    (text("display_name").or_else(|| text("name")), text("about"))
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;

    use super::*;

    fn signed(keys: &Keys, kind: u16, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn a_ship_post_is_the_makers_marked_note_or_app_event() {
        let maker = Keys::generate();
        let stranger = Keys::generate();
        let a = format!("30617:{}:tidepool", maker.public_key().to_hex());
        let site = format!("35128:{}:tidepool", maker.public_key().to_hex());
        let repo = RepoAddress::parse(&a).unwrap();
        let is_maker = |_: &RepoAddress, key: &PublicKey| *key == maker.public_key();

        let shipped = signed(&maker, 1, &[&["a", &a], &["t", "Ship"]]);
        assert_eq!(ship_repo(&shipped, is_maker), Some(repo.clone()));
        let with_media = signed(
            &maker,
            1,
            &[&["a", &a], &["imeta", "url https://cdn.test/demo.mp4"]],
        );
        assert_eq!(ship_repo(&with_media, is_maker), Some(repo.clone()));
        // Unmarked, or by someone who does not keep the repo: not a launch.
        assert_eq!(ship_repo(&signed(&maker, 1, &[&["a", &a]]), is_maker), None);
        let fan = signed(&stranger, 1, &[&["a", &a], &["t", "ship"]]);
        assert_eq!(ship_repo(&fan, is_maker), None);

        let app = signed(
            &maker,
            31_990,
            &[&["a", &a], &["a", &site], &["t", "shakespeare"]],
        );
        assert_eq!(ship_repo(&app, is_maker), Some(repo));
        assert_eq!(
            ship_repo(&signed(&maker, 31_990, &[&["a", &a]]), is_maker),
            None
        );
        assert!(app_site(&app).is_some());
    }

    #[test]
    fn a_source_names_the_repo_by_owner_and_d() {
        let owner = Keys::generate().public_key();
        let repo = RepoAddress::new(owner, "my repo").unwrap();
        let npub = npub(&owner);
        assert!(source_names(
            &format!("nostr://{npub}/relay.test/my%20repo"),
            &repo,
            None
        ));
        assert!(source_names(
            &format!("nostr://{npub}/my%20repo"),
            &repo,
            None
        ));
        assert!(source_names(
            "nostr://maya@tide.dev/my%20repo",
            &repo,
            Some("maya@tide.dev")
        ));
        assert!(!source_names(
            "nostr://maya@tide.dev/my%20repo",
            &repo,
            None
        ));
        assert!(!source_names(&format!("nostr://{npub}/other"), &repo, None));
        assert!(!source_names(
            &format!("https://{npub}/my%20repo"),
            &repo,
            None
        ));
        assert!(!source_names(&format!("nostr://{npub}"), &repo, None));
        // An naddr names it too, and `_@domain` is the domain.
        let naddr =
            nostr::nips::nip19::ToBech32::to_bech32(&nostr::nips::nip19::Nip19Coordinate::new(
                nostr::nips::nip01::Coordinate::new(Kind::from(30_617), owner)
                    .identifier("my repo"),
                Vec::<nostr::types::RelayUrl>::new(),
            ))
            .unwrap();
        assert!(source_names(&format!("nostr://{naddr}"), &repo, None));
        assert!(source_names(
            "nostr://tide.dev/relay.test/my%20repo",
            &repo,
            Some("_@tide.dev")
        ));
        assert!(!source_names("nostr://a/b/c/my%20repo", &repo, None));
    }

    #[test]
    fn a_site_coordinate_reads_back_as_the_site() {
        let author = Keys::generate().public_key();
        for site in [
            Site {
                author,
                name: Some("blog".to_owned()),
            },
            Site { author, name: None },
        ] {
            assert_eq!(Site::parse(&site.coordinate()), Some(site.clone()));
        }
        assert!(Site::parse(&format!("15128:{}", author.to_hex())).is_some());
        assert_eq!(Site::parse(&format!("35128:{}:", author.to_hex())), None);
        assert_eq!(Site::parse(&format!("30617:{}:x", author.to_hex())), None);
    }

    #[test]
    fn a_source_url_with_an_npub_is_read_as_its_repo() {
        let owner = Keys::generate().public_key();
        let repo = RepoAddress::new(owner, "my repo").unwrap();
        let npub = npub(&owner);
        for url in [
            format!("nostr://{npub}/relay.test/my%20repo"),
            format!("nostr://{npub}/my%20repo/"),
        ] {
            assert_eq!(source_repo(&url), Some(repo.clone()), "{url}");
        }
        assert_eq!(source_repo("nostr://maya@tide.dev/my%20repo"), None);
        assert_eq!(source_repo(&format!("nostr://{npub}")), None);
        assert_eq!(source_repo("https://github.com/x/y"), None);
    }

    #[test]
    fn a_site_is_addressed_as_a_gateway_serves_it() {
        let author = Keys::generate().public_key();
        let named = Site {
            author,
            name: Some("tidepool".to_owned()),
        };
        let url = named.url("https://nsite.test/").unwrap();
        let label = url
            .strip_prefix("https://")
            .unwrap()
            .split('.')
            .next()
            .unwrap();
        assert_eq!(label.len(), 50 + "tidepool".len());
        assert!(label.ends_with("tidepool"));
        let root = Site { author, name: None };
        assert_eq!(
            root.url("https://nsite.test").unwrap(),
            format!("https://{}.nsite.test/", npub(&author))
        );
        for bad in ["Has-Caps", "much-too-long-name", "trailing-", ""] {
            let site = Site {
                author,
                name: Some(bad.to_owned()),
            };
            assert_eq!(site.url("https://nsite.test"), None, "{bad}");
        }
        assert_eq!(root.url("http://nsite.test"), None);
    }

    #[test]
    fn base36_pads_a_key_to_fifty_digits() {
        assert_eq!(base36(&[0; 32]), "0".repeat(50));
        assert_eq!(base36(&[0, 0, 36]).trim_start_matches('0'), "10");
        assert_eq!(base36(&[0xff; 32]).len(), 50);
    }
}
