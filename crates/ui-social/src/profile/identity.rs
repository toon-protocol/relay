//! Who a key says it is: its kind 0 profile with the NIP-24 extras, its
//! NIP-39 external identities and its NIP-38 statuses. Pure functions over
//! events, no I/O.
//!
//! Everything here is a claim. The relay never fetches a NIP-05
//! `.well-known` document, a gist or a post that would prove one (ADR 0001),
//! so a page draws each as the key's own word and links to where a reader
//! can check it.

use nostr::event::{Event, Kind};
use nostr::key::PublicKey;
use relay_ui::chrome::nip05_identifier;
use ui_nip34::repo::{RepoAddress, d_tag, first_tag, newest};

use crate::media::Media;

/// NIP-38 user status.
pub(crate) const USER_STATUS: u16 = 30_315;
/// NIP-39 external identities.
pub(crate) const EXTERNAL_IDENTITIES: u16 = 10_011;

/// What a kind 0 profile says, read leniently: a field of the wrong type is
/// left out rather than failing the profile.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Metadata {
    pub(crate) name: Option<String>,
    /// The `name` when a `display_name` is shown instead.
    pub(crate) handle: Option<String>,
    pub(crate) about: Option<String>,
    /// An `http(s)` URL.
    pub(crate) website: Option<String>,
    /// An `https` image, for the browser to load.
    pub(crate) banner: Option<Media>,
    /// NIP-24 `bot`: the key's content is partly or wholly automated.
    pub(crate) bot: bool,
    /// The NIP-05 identifier it claims, unverified.
    pub(crate) nip05: Option<String>,
    /// NIP-39 `i` tags on the profile, the form before kind 10011.
    pub(crate) identities: Vec<Identity>,
}

/// The newest kind 0 of `key` among `events`, read. A pure function.
pub(crate) fn metadata(key: &PublicKey, events: &[Event]) -> Metadata {
    let Some(profile) = newest(
        events
            .iter()
            .filter(|event| event.pubkey == *key && event.kind == Kind::Metadata),
    ) else {
        return Metadata::default();
    };
    let mut read = Metadata {
        identities: identities(profile),
        ..Metadata::default()
    };
    let Ok(serde_json::Value::Object(fields)) = serde_json::from_str(&profile.content) else {
        return read;
    };
    let text = |field: &str| {
        fields
            .get(field)
            .and_then(|value| value.as_str())
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_owned)
    };
    let display = text("display_name");
    let name = text("name");
    read.name = display.clone().or_else(|| name.clone());
    read.handle = name.filter(|name| display.as_ref().is_some_and(|shown| shown != name));
    read.about = text("about");
    read.website = text("website").filter(|url| web_url(url));
    read.banner = text("banner").and_then(|url| Media::new(&url, Some("image/*")));
    read.bot = fields.get("bot").and_then(serde_json::Value::as_bool) == Some(true);
    read.nip05 = text("nip05").and_then(|claim| nip05_identifier(&claim));
    read
}

/// Whether `url` is an `http(s)` URL a link can carry as it is.
fn web_url(url: &str) -> bool {
    (url.starts_with("https://") || url.starts_with("http://"))
        && !url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '<' | '>'))
}

/// A NIP-39 external identity: a platform account the key claims, and the
/// proof a reader can check by hand.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Identity {
    pub(crate) platform: String,
    pub(crate) identity: String,
    pub(crate) proof: String,
}

/// Whether `text` is made of `allowed` bytes and ASCII alphanumerics only,
/// so it can be written into a URL path as it is.
fn plain(text: &str, allowed: &[u8]) -> bool {
    !text.is_empty()
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || allowed.contains(&b))
}

impl Identity {
    /// What a reader calls the platform.
    pub(crate) fn label(&self) -> String {
        match self.platform.as_str() {
            "github" => "GitHub".to_owned(),
            "twitter" => "X".to_owned(),
            "mastodon" => "Mastodon".to_owned(),
            "telegram" => "Telegram".to_owned(),
            other => other.to_owned(),
        }
    }

    /// The account's page, for the platforms NIP-39 names.
    pub(crate) fn profile_url(&self) -> Option<String> {
        let id = &self.identity;
        match self.platform.as_str() {
            "github" if plain(id, b"-") => Some(format!("https://github.com/{id}")),
            "twitter" if plain(id, b"_") => Some(format!("https://twitter.com/{id}")),
            "mastodon" if plain(id, b"-._/@") && id.contains("/@") => Some(format!("https://{id}")),
            _ => None,
        }
    }

    /// Where the proof lives, as NIP-39 spells it per platform.
    pub(crate) fn proof_url(&self) -> Option<String> {
        let (id, proof) = (&self.identity, &self.proof);
        if !plain(proof, b"-_/") {
            return None;
        }
        match self.platform.as_str() {
            "github" if plain(id, b"-") => Some(format!("https://gist.github.com/{id}/{proof}")),
            "twitter" if plain(id, b"_") => {
                Some(format!("https://twitter.com/{id}/status/{proof}"))
            }
            "mastodon" if plain(id, b"-._/@") && id.contains("/@") => {
                Some(format!("https://{id}/{proof}"))
            }
            "telegram" => Some(format!("https://t.me/{proof}")),
            _ => None,
        }
    }
}

/// The `i` tags of `event`, `["i", "<platform>:<identity>", "<proof>"]`,
/// once each. A pure function.
pub(crate) fn identities(event: &Event) -> Vec<Identity> {
    let mut found: Vec<Identity> = Vec::new();
    for tag in event.tags.iter().map(|tag| tag.as_slice()) {
        let [name, claim, rest @ ..] = tag else {
            continue;
        };
        if name != "i" {
            continue;
        }
        let Some((platform, identity)) = claim.split_once(':') else {
            continue;
        };
        let platform = platform.trim().to_lowercase();
        let identity = identity.trim().to_owned();
        if platform.is_empty() || identity.is_empty() {
            continue;
        }
        let identity = Identity {
            platform,
            identity,
            proof: rest
                .first()
                .map(|p| p.trim().to_owned())
                .unwrap_or_default(),
        };
        if !found
            .iter()
            .any(|held| held.platform == identity.platform && held.identity == identity.identity)
        {
            found.push(identity);
        }
    }
    found
}

/// A NIP-38 status a key has set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Status {
    /// The status type: `general`, `music`, or another the client chose.
    pub(crate) kind: String,
    pub(crate) text: String,
    /// The `r` link, if it is `http(s)`.
    pub(crate) link: Option<String>,
    /// The repo it names in an `a` tag, if any: "shipping <repo>".
    pub(crate) repo: Option<RepoAddress>,
}

/// The statuses of `key` that stand at `now`: per status type the newest,
/// unless it was cleared (empty content) or its NIP-40 `expiration` has
/// passed. `general` first, then by type. A pure function.
pub(crate) fn statuses(key: &PublicKey, events: &[Event], now: i64) -> Vec<Status> {
    let mut kinds: Vec<&str> = events
        .iter()
        .filter(|event| event.pubkey == *key && event.kind == Kind::from(USER_STATUS))
        .map(d_tag)
        .collect();
    kinds.sort_by_key(|kind| (*kind != "general", *kind));
    kinds.dedup();
    let mut standing = Vec::new();
    for kind in kinds {
        let Some(status) = newest(events.iter().filter(|event| {
            event.pubkey == *key && event.kind == Kind::from(USER_STATUS) && d_tag(event) == kind
        })) else {
            continue;
        };
        let expired = first_tag(status, "expiration")
            .and_then(|at| at.trim().parse::<i64>().ok())
            .is_some_and(|at| at <= now);
        let text = status.content.trim();
        if expired || text.is_empty() {
            continue;
        }
        standing.push(Status {
            kind: kind.to_owned(),
            text: text.to_owned(),
            link: first_tag(status, "r")
                .filter(|url| web_url(url))
                .map(str::to_owned),
            repo: RepoAddress::tagged(status, "a").into_iter().next(),
        });
    }
    standing
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;
    use nostr::types::Timestamp;

    use super::*;

    fn signed(keys: &Keys, kind: u16, at: u64, content: &str, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(kind), content)
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .custom_created_at(Timestamp::from(at))
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn a_profile_reads_its_nip24_extras_and_drops_what_a_page_cannot_draw() {
        let keys = Keys::generate();
        let events = [
            signed(&keys, 0, 1, r#"{"name":"old"}"#, &[]),
            signed(
                &keys,
                0,
                2,
                r#"{"name":"shipbot","display_name":"Ship Bot","about":"I write release notes",
                   "website":"https://ship.example","banner":"https://cdn.example/b.png",
                   "bot":true,"nip05":"Bot@Ship.Example"}"#,
                &[&["i", "github:shipbot", "abc123"]],
            ),
        ];
        let read = metadata(&keys.public_key(), &events);
        assert_eq!(read.name.as_deref(), Some("Ship Bot"));
        assert_eq!(read.handle.as_deref(), Some("shipbot"));
        assert_eq!(read.website.as_deref(), Some("https://ship.example"));
        assert_eq!(
            read.banner.as_ref().map(Media::url),
            Some("https://cdn.example/b.png")
        );
        assert!(read.bot);
        assert_eq!(read.nip05.as_deref(), Some("bot@ship.example"));
        assert_eq!(read.identities.len(), 1);

        let odd = [signed(
            &keys,
            0,
            3,
            r#"{"name":"x","website":"javascript:alert(1)","banner":"http://cdn.example/b.png","bot":"yes"}"#,
            &[],
        )];
        let read = metadata(&keys.public_key(), &odd);
        assert_eq!(read.website, None);
        assert_eq!(read.banner, None, "only an https banner is drawn");
        assert!(!read.bot, "bot is a boolean");
        assert_eq!(read.handle, None);
        assert_eq!(metadata(&keys.public_key(), &[]), Metadata::default());
    }

    #[test]
    fn an_identity_links_to_its_account_and_its_proof() {
        let keys = Keys::generate();
        let list = signed(
            &keys,
            EXTERNAL_IDENTITIES,
            1,
            "",
            &[
                &["i", "github:mayaokafor", "9721ce4ee4fceb91c9711ca2a6c9a5ab"],
                &["i", "twitter:mayabuilds", "1619358434134196225"],
                &[
                    "i",
                    "mastodon:bitcoinhackers.org/@maya",
                    "109775066355589974",
                ],
                &["i", "telegram:1087295469", "nostrdirectory/770"],
                &["i", "github:mayaokafor", "again"],
                &["i", "github:bad/../path", "x"],
                &["i", "nocolon"],
                &["p", "github:x"],
            ],
        );
        let found = identities(&list);
        assert_eq!(found.len(), 5);
        assert_eq!(
            found[0].proof_url().as_deref(),
            Some("https://gist.github.com/mayaokafor/9721ce4ee4fceb91c9711ca2a6c9a5ab")
        );
        assert_eq!(
            found[0].profile_url().as_deref(),
            Some("https://github.com/mayaokafor")
        );
        assert_eq!(
            found[1].proof_url().as_deref(),
            Some("https://twitter.com/mayabuilds/status/1619358434134196225")
        );
        assert_eq!(found[1].label(), "X");
        assert_eq!(
            found[2].proof_url().as_deref(),
            Some("https://bitcoinhackers.org/@maya/109775066355589974")
        );
        assert_eq!(
            found[3].proof_url().as_deref(),
            Some("https://t.me/nostrdirectory/770")
        );
        assert_eq!(found[3].profile_url(), None);
        assert_eq!(found[4].profile_url(), None, "a path is not an account");
        assert_eq!(found[4].proof_url(), None);
    }

    #[test]
    fn a_status_stands_until_it_is_cleared_replaced_or_expires() {
        let keys = Keys::generate();
        let repo = format!("30617:{}:tidepool", keys.public_key().to_hex());
        let now = 1_000;
        let events = [
            signed(&keys, USER_STATUS, 1, "old", &[&["d", "general"]]),
            signed(
                &keys,
                USER_STATUS,
                2,
                "Shipping wind overlays",
                &[&["d", "general"], &["a", &repo], &["expiration", "2000"]],
            ),
            signed(
                &keys,
                USER_STATUS,
                3,
                "A song",
                &[
                    &["d", "music"],
                    &["r", "https://music.example/x"],
                    &["expiration", "999"],
                ],
            ),
            signed(&keys, USER_STATUS, 4, "", &[&["d", "away"]]),
            signed(
                &keys,
                USER_STATUS,
                5,
                "At the dock",
                &[&["d", "boat"], &["r", "javascript:x"]],
            ),
        ];
        let standing = statuses(&keys.public_key(), &events, now);
        assert_eq!(standing.len(), 2, "{standing:?}");
        assert_eq!(standing[0].kind, "general");
        assert_eq!(standing[0].text, "Shipping wind overlays");
        assert_eq!(standing[0].repo, RepoAddress::parse(&repo));
        assert_eq!(standing[1].kind, "boat");
        assert_eq!(standing[1].link, None);
        assert!(statuses(&keys.public_key(), &events, 2_000).len() == 1);
    }
}
