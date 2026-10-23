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
//! A kind 1 ship's live demo is a NIP-5A site manifest signed by an
//! authorized author whose `source` tag is the repo's `nostr://` URL. A demo
//! is linked only through the operator's nsite gateway
//! (`TOON_UI_NSITE_GATEWAY`): the relay serves nothing of the site.

use nostr::event::{Event, Kind};
use nostr::filter::Filter;
use nostr::key::PublicKey;
use relay_ui::ReadOnlyStore;
use relay_ui::chrome::npub;
use ui_nip34::card::RepoCard;
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
    fn of(event: &Event) -> Option<Self> {
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

    /// The site's address on the gateway at `gateway` (an `https` URL), if
    /// its name can be a DNS label: a root site is `<npub>.<host>`, a named
    /// one `<pubkey in base36><name>.<host>`, as NIP-5A gateways serve them.
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

/// The site an app event names in an `a` tag `35128:<pubkey>:<name>`.
pub(crate) fn app_site(event: &Event) -> Option<Site> {
    if event.kind != Kind::from(APP_HANDLER) {
        return None;
    }
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find_map(|tag| match tag {
            [name, value, ..] if name == "a" => {
                let mut parts = value.splitn(3, ':');
                let (kind, author, d) = (parts.next()?, parts.next()?, parts.next()?);
                (kind == SITE_NAMED.to_string()).then_some(())?;
                Some(Site {
                    author: PublicKey::from_hex(author).ok()?,
                    name: Some(d.to_owned()),
                })
            }
            _ => None,
        })
}

/// Whether a NIP-5A `source` tag names `repo`: a `nostr://` URL whose first
/// part is the owner's npub (or the NIP-05 name the owner claims) and whose
/// last part is the repo's `d`. A pure function.
pub(crate) fn source_names(source: &str, repo: &RepoAddress, nip05: Option<&str>) -> bool {
    let Some(rest) = source.trim().strip_prefix("nostr://") else {
        return false;
    };
    let parts: Vec<&str> = rest.trim_end_matches('/').split('/').collect();
    let (Some(who), Some(last)) = (parts.first(), parts.last()) else {
        return false;
    };
    if parts.len() < 2 {
        return false;
    }
    let who_matches =
        *who == npub(repo.owner()) || nip05.is_some_and(|claim| who.eq_ignore_ascii_case(claim));
    who_matches && percent_decode(last) == repo.d()
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

/// The live demo of `card`'s repo: the newest site manifest an authorized
/// author signed whose `source` names the repo.
pub(crate) async fn demo(store: &ReadOnlyStore, card: &RepoCard) -> Option<Site> {
    let mut sites = store
        .query(
            Filter::new()
                .kinds([Kind::from(SITE_NAMED), Kind::from(SITE_ROOT)])
                .authors(card.authorized().to_vec())
                .limit(200),
        )
        .await;
    sites.sort_by(|a, b| (b.created_at, a.id).cmp(&(a.created_at, b.id)));
    let nip05 = card.profiles().nip05(card.address().owner());
    sites
        .iter()
        .filter(|site| card.is_maker(&site.pubkey))
        .find(|site| {
            site.tags
                .iter()
                .map(|tag| tag.as_slice())
                .any(|tag| matches!(tag, [name, value, ..] if name == "source" && source_names(value, card.address(), nip05)))
        })
        .and_then(Site::of)
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
