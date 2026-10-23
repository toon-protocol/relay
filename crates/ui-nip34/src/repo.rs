//! What a NIP-34 repository is, read from its events: its address, its
//! owner's announcement, who may move its refs (ADR 0002) and the refs they
//! moved. Pure functions over events, with no I/O, so every reader — the
//! `/repo` pages, the repo card, the `/social` pages — resolves the same
//! repository the same way.

use nostr::event::{Event, Kind};
use nostr::key::PublicKey;
use nostr::nips::nip01::Coordinate;
use nostr::nips::nip19::{FromBech32, Nip19Coordinate};

use relay_ui::chrome::{npub, percent_encode};

/// NIP-34 repository announcement.
pub const ANNOUNCEMENT: u16 = 30_617;
/// NIP-34 repository state.
pub const STATE: u16 = 30_618;

/// A repository's address, `30617:<owner>:<d>`: the one name every social
/// act on a repo points at. It is only built from an address that names
/// kind 30617 and a non-empty `d`, so holding one means it is a repo's.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RepoAddress {
    owner: PublicKey,
    d: String,
}

impl RepoAddress {
    /// The repo `d` that `owner` announces, if `d` is not empty.
    pub fn new(owner: PublicKey, d: &str) -> Option<Self> {
        (!d.is_empty()).then(|| Self {
            owner,
            d: d.to_owned(),
        })
    }

    /// The address an `a` tag value spells, `30617:<hex>:<d>`.
    pub fn parse(coordinate: &str) -> Option<Self> {
        let mut parts = coordinate.splitn(3, ':');
        let (kind, owner, d) = (parts.next()?, parts.next()?, parts.next()?);
        if kind.parse::<u16>().ok()? != ANNOUNCEMENT {
            return None;
        }
        Self::new(PublicKey::from_hex(owner).ok()?, d)
    }

    /// The address an `naddr` spells, with or without `nostr:` before it, if
    /// it names kind 30617.
    pub fn from_naddr(text: &str) -> Option<Self> {
        let text = text.strip_prefix("nostr:").unwrap_or(text);
        let coordinate: Coordinate = Nip19Coordinate::from_bech32(text).ok()?.coordinate;
        (coordinate.kind == Kind::from(ANNOUNCEMENT))
            .then(|| Self::new(coordinate.public_key, &coordinate.identifier))
            .flatten()
    }

    /// The address of a kind 30617 announcement.
    pub fn of(announcement: &Event) -> Option<Self> {
        (announcement.kind == Kind::from(ANNOUNCEMENT))
            .then(|| Self::new(announcement.pubkey, d_tag(announcement)))
            .flatten()
    }

    /// The repos `event` names in a tag called `name` (`a`, or a NIP-22
    /// root's `A`), once each, in order.
    pub fn tagged(event: &Event, name: &str) -> Vec<Self> {
        let mut found: Vec<Self> = Vec::new();
        for tag in event.tags.iter().map(|tag| tag.as_slice()) {
            if let [tag_name, value, ..] = tag
                && tag_name == name
                && let Some(address) = Self::parse(value)
                && !found.contains(&address)
            {
                found.push(address);
            }
        }
        found
    }

    /// The owner's key.
    pub fn owner(&self) -> &PublicKey {
        &self.owner
    }

    /// The `d` tag: the repository's identifier.
    pub fn d(&self) -> &str {
        &self.d
    }

    /// `30617:<owner hex>:<d>`, as an `a` tag carries it.
    pub fn coordinate(&self) -> String {
        format!("{ANNOUNCEMENT}:{}:{}", self.owner.to_hex(), self.d)
    }

    /// `<npub>/<d>`, percent-encoded: the path an app puts after its prefix.
    pub fn path_segments(&self) -> String {
        format!("{}/{}", npub(&self.owner), percent_encode(&self.d))
    }

    /// The repository's `/repo` page.
    pub fn repo_path(&self) -> String {
        format!("{}/{}", crate::PREFIX, self.path_segments())
    }
}

/// The values of the first tag called `name`, after the name.
pub fn tag_values<'a>(event: &'a Event, name: &str) -> &'a [String] {
    event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find(|tag| tag.first().is_some_and(|first| first == name))
        .map_or(&[], |tag| &tag[1..])
}

/// The first value of the first tag called `name`.
pub fn first_tag<'a>(event: &'a Event, name: &str) -> Option<&'a str> {
    tag_values(event, name).first().map(String::as_str)
}

/// The event's `d` tag, or empty.
pub fn d_tag(event: &Event) -> &str {
    first_tag(event, "d").unwrap_or("")
}

/// Whether `candidate` is a newer replacement than `held`: later, and on a
/// `created_at` tie the lowest event id (ADR 0002).
pub fn supersedes(candidate: &Event, held: &Event) -> bool {
    (candidate.created_at, held.id) > (held.created_at, candidate.id)
}

/// The newest of `events`, or none.
pub fn newest<'a>(events: impl IntoIterator<Item = &'a Event>) -> Option<&'a Event> {
    events
        .into_iter()
        .fold(None, |held: Option<&Event>, candidate| match held {
            Some(held) if !supersedes(candidate, held) => Some(held),
            _ => Some(candidate),
        })
}

/// The name a repo goes by: its `name` tag, else its `d`.
pub fn repo_name(announcement: &Event) -> &str {
    first_tag(announcement, "name")
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| d_tag(announcement))
}

/// The repo's `t` topics, once each, in order.
pub fn topics(announcement: &Event) -> Vec<&str> {
    let mut found: Vec<&str> = Vec::new();
    for tag in announcement.tags.iter().map(|tag| tag.as_slice()) {
        if let [name, value, ..] = tag
            && name == "t"
            && !value.trim().is_empty()
            && !found.contains(&value.as_str())
        {
            found.push(value);
        }
    }
    found
}

/// The refs a repo state names, `(name, commit)`, in both shapes a state is
/// written in (rig's rule, rig#156): NIP-34's `["refs/heads/main", <sha>]`,
/// and the legacy `["r", "refs/heads/main", <sha>]` that every repo pushed by
/// the rig CLI carries. When both name a ref, NIP-34's wins. A peeled tag
/// (`refs/tags/v1^{}`) is not a ref. A pure function.
pub fn refs(state: &Event) -> Vec<(&str, &str)> {
    let is_ref = |name: &str| {
        (name.starts_with("refs/heads/") || name.starts_with("refs/tags/"))
            && !name.ends_with("^{}")
    };
    let mut found: Vec<(&str, &str)> = Vec::new();
    for tag in state.tags.iter().map(|tag| tag.as_slice()) {
        if let [name, commit, ..] = tag
            && is_ref(name)
            && !found.iter().any(|(held, _)| held == name)
        {
            found.push((name, commit));
        }
    }
    for tag in state.tags.iter().map(|tag| tag.as_slice()) {
        if let [marker, name, commit, ..] = tag
            && marker == "r"
            && is_ref(name)
            && !found.iter().any(|(held, _)| held == name)
        {
            found.push((name, commit));
        }
    }
    found
}

/// The branch a repo state's `HEAD` names: NIP-34's `["HEAD", "ref:
/// refs/heads/main"]`, else rig's `["r", "HEAD", …]`, else `main` as rig-web
/// assumed. A pure function.
pub fn head_branch(state: &Event) -> String {
    let nip = first_tag(state, "HEAD");
    let legacy = state
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find_map(|tag| match tag {
            [marker, head, value, ..] if marker == "r" && head == "HEAD" => Some(value.as_str()),
            _ => None,
        });
    nip.or(legacy)
        .map(|head| {
            let head = head.strip_prefix("ref:").unwrap_or(head).trim();
            head.strip_prefix("refs/heads/").unwrap_or(head).to_owned()
        })
        .filter(|head| !head.is_empty())
        .unwrap_or_else(|| "main".to_owned())
}

/// The pubkeys whose repo state is honoured: the owner and the maintainers
/// in the owner's own announcement (ADR 0002). A pure function.
pub fn authorized_authors(announcement: &Event) -> Vec<PublicKey> {
    let mut authors = vec![announcement.pubkey];
    for value in tag_values(announcement, "maintainers") {
        if let Ok(key) = PublicKey::from_hex(value)
            && !authors.contains(&key)
        {
            authors.push(key);
        }
    }
    authors
}

/// The honoured repo state among `states` for `announcement`: the newest
/// signed by an authorized author (ADR 0002). A pure function.
pub fn honoured_state<'a>(announcement: &Event, states: &'a [Event]) -> Option<&'a Event> {
    let authors = authorized_authors(announcement);
    let d = d_tag(announcement);
    newest(
        states
            .iter()
            .filter(|state| state.kind == Kind::from(STATE))
            .filter(|state| authors.contains(&state.pubkey) && d_tag(state) == d),
    )
}

/// The repo's announcement among `announcements`: the newest `owner` signed
/// for `d`. Another key's announcement for the same `d` never stands in for
/// it, so its `maintainers` are never honoured (ADR 0002). A pure function.
pub fn owner_announcement<'a>(
    owner: &PublicKey,
    d: &str,
    announcements: &'a [Event],
) -> Option<&'a Event> {
    newest(
        announcements
            .iter()
            .filter(|event| event.kind == Kind::from(ANNOUNCEMENT))
            .filter(|event| event.pubkey == *owner && d_tag(event) == d),
    )
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;
    use nostr::nips::nip19::ToBech32;

    use super::*;

    #[test]
    fn an_address_names_a_repo_or_is_nothing() {
        let key = Keys::generate().public_key();
        let hex = key.to_hex();
        let address = RepoAddress::parse(&format!("30617:{hex}:my-repo")).expect("parses");
        assert_eq!(address.owner(), &key);
        assert_eq!(address.d(), "my-repo");
        assert_eq!(address.coordinate(), format!("30617:{hex}:my-repo"));
        // A `d` may hold colons: only the first two split.
        assert_eq!(
            RepoAddress::parse(&format!("30617:{hex}:a:b"))
                .expect("parses")
                .d(),
            "a:b"
        );
        for bad in [
            format!("30618:{hex}:my-repo"),
            format!("30617:{hex}:"),
            "30617:nothex:my-repo".to_owned(),
            "30617".to_owned(),
        ] {
            assert_eq!(RepoAddress::parse(&bad), None, "{bad}");
        }
    }

    #[test]
    fn an_naddr_of_another_kind_is_not_a_repo() {
        let key = Keys::generate().public_key();
        let naddr = |kind: u16| {
            let coordinate = Coordinate::new(Kind::from(kind), key).identifier("r");
            Nip19Coordinate::new(coordinate, Vec::<nostr::types::RelayUrl>::new())
                .to_bech32()
                .expect("encodes")
        };
        let repo = RepoAddress::from_naddr(&format!("nostr:{}", naddr(30_617))).expect("a repo");
        assert_eq!(repo.d(), "r");
        assert_eq!(RepoAddress::from_naddr(&naddr(30_023)), None);
    }

    #[test]
    fn a_tag_names_each_repo_once() {
        let key = Keys::generate();
        let coordinate = format!("30617:{}:r", key.public_key().to_hex());
        let tags: Vec<Tag> = [
            vec!["a", coordinate.as_str()],
            vec!["a", coordinate.as_str(), "wss://x"],
            vec!["a", "30023:00:x"],
            vec!["A", coordinate.as_str()],
        ]
        .into_iter()
        .map(|parts| Tag::parse(parts).unwrap())
        .collect();
        let event = EventBuilder::new(Kind::TextNote, "")
            .tags(tags)
            .finalize(&key)
            .unwrap();
        assert_eq!(RepoAddress::tagged(&event, "a").len(), 1);
        assert_eq!(RepoAddress::tagged(&event, "A").len(), 1);
    }
}
