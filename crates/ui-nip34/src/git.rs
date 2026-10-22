//! The tree and blob pages (#285): a page layout the browser fills.
//!
//! The relay never fetches a git object (ADR 0001). A page carries the
//! resolved commit and, for each object the repo's events name, the ordered
//! candidate URLs the visitor's browser tries; `ui-nip34-web` fetches,
//! verifies and renders. The layout is JSON in a `data-` attribute, so the
//! page carries no inline script.

use std::collections::BTreeMap;

use axum::extract::{Path, State};
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::Response;
use nostr::event::{Event, Kind};
use nostr::filter::Filter;
use nostr::key::PublicKey;
use nostr::nips::nip19::FromBech32;
use serde_json::json;

use crate::{
    ANNOUNCEMENT, PREFIX, Pages, SCRIPT, STATE, UNAVAILABLE, authorized_authors, d_tag, escape,
    honoured_state, html, newest, not_found, percent_encode, repo_name, repo_path, tag_values,
};

/// What a git page may load: its own origin, its own WebAssembly, and `https`
/// anywhere to fetch objects from. Still no inline script.
pub const CONTENT_SECURITY_POLICY_VALUE: &str =
    "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'; connect-src https:";

/// The operator's default places for objects (`TOON_UI_ARWEAVE_GATEWAYS`,
/// `TOON_UI_BLOB_SERVERS`), tried after those the repo's events name.
#[derive(Clone, Debug, Default)]
pub struct Defaults {
    pub arweave_gateways: Vec<String>,
    pub blob_servers: Vec<String>,
}

/// Which page: a directory listing or a file.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum PageKind {
    Tree,
    Blob,
}

impl PageKind {
    fn name(self) -> &'static str {
        match self {
            Self::Tree => "tree",
            Self::Blob => "blob",
        }
    }
}

/// An `https` URL a browser may fetch: no whitespace or control characters,
/// and something after the scheme.
fn https_url(url: &str) -> Option<&str> {
    let rest = url.strip_prefix("https://")?;
    (!rest.is_empty()
        && !rest.starts_with('/')
        && !url.chars().any(|c| c.is_whitespace() || c.is_control()))
    .then_some(url)
}

/// `base` and `leaf` joined by exactly one slash.
fn join(base: &str, leaf: &str) -> String {
    format!("{}/{leaf}", base.trim_end_matches('/'))
}

fn is_object_id(text: &str) -> bool {
    text.len() == 40 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// An Arweave transaction id: base64url characters.
fn is_transaction_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 64
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn is_blob_id(text: &str) -> bool {
    text.len() == 64 && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// The ordered candidate URLs for each object the repo's events name. A pure
/// function.
///
/// Per object, in order:
/// 1. the 30618's `["arweave", <sha1>, <txId>, <gateway>…]` tag, against the
///    gateways that tag names;
/// 2. its `["blossom", <sha1>, <blob id>]` tag, against the announcement's
///    `blossoms` servers;
/// 3. the same Arweave transaction against the operator's gateways;
/// 4. the same blob against the operator's Blossom servers.
///
/// Only `https` URLs are listed, and each once.
pub(crate) fn candidates(
    announcement: &Event,
    state: &Event,
    defaults: &Defaults,
) -> BTreeMap<String, Vec<String>> {
    #[derive(Default)]
    struct Held<'a> {
        arweave: Option<(&'a str, Vec<&'a str>)>,
        blossom: Option<&'a str>,
    }
    let mut held: BTreeMap<&str, Held<'_>> = BTreeMap::new();
    for tag in state.tags.iter().map(|tag| tag.as_slice()) {
        match tag {
            [name, object, tx, gateways @ ..]
                if name == "arweave" && is_object_id(object) && is_transaction_id(tx) =>
            {
                held.entry(object)
                    .or_default()
                    .arweave
                    .get_or_insert((tx, gateways.iter().filter_map(|g| https_url(g)).collect()));
            }
            [name, object, blob, ..]
                if name == "blossom" && is_object_id(object) && is_blob_id(blob) =>
            {
                held.entry(object).or_default().blossom.get_or_insert(blob);
            }
            _ => {}
        }
    }
    let servers: Vec<&str> = tag_values(announcement, "blossoms")
        .iter()
        .filter_map(|server| https_url(server))
        .collect();
    let operator_gateways = defaults
        .arweave_gateways
        .iter()
        .filter_map(|g| https_url(g));
    let operator_servers = defaults.blob_servers.iter().filter_map(|s| https_url(s));
    let operator_gateways: Vec<&str> = operator_gateways.collect();
    let operator_servers: Vec<&str> = operator_servers.collect();

    let mut out = BTreeMap::new();
    for (object, held) in held {
        let mut urls: Vec<String> = Vec::new();
        let mut add = |url: String| {
            if !urls.contains(&url) {
                urls.push(url);
            }
        };
        if let Some((tx, gateways)) = &held.arweave {
            gateways.iter().for_each(|gateway| add(join(gateway, tx)));
        }
        if let Some(blob) = held.blossom {
            servers.iter().for_each(|server| add(join(server, blob)));
        }
        if let Some((tx, _)) = &held.arweave {
            operator_gateways
                .iter()
                .for_each(|gateway| add(join(gateway, tx)));
        }
        if let Some(blob) = held.blossom {
            operator_servers
                .iter()
                .for_each(|server| add(join(server, blob)));
        }
        out.insert(object.to_owned(), urls);
    }
    out
}

/// A ref, its commit and the path below it, from the segments after
/// `/tree/` or `/blob/`. Ref names hold slashes, so the longest name the
/// state holds wins; a 40-hex commit id is a ref too, but a ref of that name
/// takes precedence. A pure function.
pub(crate) fn resolve<'a>(
    state: &Event,
    segments: &'a [String],
) -> Option<(String, String, &'a [String])> {
    let commit_of = |name: &str| {
        ["refs/heads/", "refs/tags/"].iter().find_map(|prefix| {
            let wanted = format!("{prefix}{name}");
            state
                .tags
                .iter()
                .map(|tag| tag.as_slice())
                .find_map(|tag| match tag {
                    [tag_name, commit, ..] if *tag_name == wanted && is_object_id(commit) => {
                        Some(commit.clone())
                    }
                    _ => None,
                })
        })
    };
    (1..=segments.len()).rev().find_map(|count| {
        let name = segments[..count].join("/");
        let commit = commit_of(&name).or_else(|| {
            (count == 1 && is_object_id(&name.to_ascii_lowercase()))
                .then(|| name.to_ascii_lowercase())
        })?;
        Some((name, commit, &segments[count..]))
    })
}

pub(crate) async fn tree(
    State(pages): State<Pages>,
    Path((owner, d, rest)): Path<(String, String, String)>,
) -> Response {
    page(pages, PageKind::Tree, &owner, &d, &rest).await
}

pub(crate) async fn blob(
    State(pages): State<Pages>,
    Path((owner, d, rest)): Path<(String, String, String)>,
) -> Response {
    page(pages, PageKind::Blob, &owner, &d, &rest).await
}

fn link(href: &str, text: &str) -> String {
    format!("<a href=\"{}\">{}</a>", escape(href), escape(text))
}

async fn page(pages: Pages, kind: PageKind, owner: &str, d: &str, rest: &str) -> Response {
    let Pages {
        store,
        assets,
        defaults,
    } = pages;
    let Ok(owner) = PublicKey::from_bech32(owner) else {
        return not_found().await;
    };
    let announcements = store
        .query(
            Filter::new()
                .kind(Kind::from(ANNOUNCEMENT))
                .author(owner)
                .identifier(d.to_owned()),
        )
        .await;
    let Some(announcement) = newest(
        announcements
            .iter()
            .filter(|event| event.pubkey == owner && d_tag(event) == d),
    ) else {
        return not_found().await;
    };
    let states = store
        .query(
            Filter::new()
                .kind(Kind::from(STATE))
                .authors(authorized_authors(announcement))
                .identifier(d.to_owned()),
        )
        .await;
    let Some(state) = honoured_state(announcement, &states) else {
        return not_found().await;
    };
    let segments: Vec<String> = rest
        .split('/')
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect();
    let Some((ref_name, commit, path)) = resolve(state, &segments) else {
        return not_found().await;
    };
    // A blob page with no file is not a page.
    if kind == PageKind::Blob && path.is_empty() {
        return not_found().await;
    }

    let base = repo_path(&owner, d);
    let location = |count: usize| {
        let mut href = format!(
            "{base}/tree/{}",
            ref_name
                .split('/')
                .map(percent_encode)
                .collect::<Vec<_>>()
                .join("/")
        );
        for segment in &path[..count] {
            href.push('/');
            href.push_str(&percent_encode(segment));
        }
        href
    };
    let mut body = format!(
        "<h1>{}</h1><p>{} at <code>{}</code> (<code>{}</code>)</p><p>{}",
        link(&base, repo_name(announcement)),
        if kind == PageKind::Tree {
            "Tree"
        } else {
            "File"
        },
        escape(&ref_name),
        escape(&commit),
        link(&location(0), repo_name(announcement)),
    );
    for (index, segment) in path.iter().enumerate() {
        body.push_str(" / ");
        if index + 1 == path.len() && kind == PageKind::Blob {
            body.push_str(&escape(segment));
        } else {
            body.push_str(&link(&location(index + 1), segment));
        }
    }
    body.push_str("</p>");

    match assets.and_then(|assets| assets.url(PREFIX, SCRIPT)) {
        Some(script) => {
            let layout = json!({
                "kind": kind.name(),
                "base": base,
                "ref": ref_name,
                "commit": commit,
                "path": path,
                "objects": candidates(announcement, state, &defaults),
            });
            body.push_str(&format!(
                "<main id=\"git\" data-git=\"{}\">Loading&hellip;</main>\
                 <noscript><p>This page needs JavaScript to fetch the repository's files.</p></noscript>\
                 <script type=\"module\" src=\"{}\"></script>",
                escape(&layout.to_string()),
                escape(&script)
            ));
        }
        None => body.push_str(&format!("<p>{UNAVAILABLE}</p>")),
    }
    let title = format!("{} {}", repo_name(announcement), path.join("/"));
    let mut response = html(StatusCode::OK, &title, &body);
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CONTENT_SECURITY_POLICY_VALUE),
    );
    response
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;

    use super::*;

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const BLOB: &str = "cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc";

    fn sign(kind: u16, tags: &[&[&str]]) -> Event {
        let tags: Vec<Tag> = tags
            .iter()
            .map(|parts| Tag::parse(parts.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>()))
            .collect::<Result<_, _>>()
            .unwrap();
        EventBuilder::new(Kind::from(kind), "")
            .tags(tags)
            .finalize(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn candidates_run_from_the_events_to_the_operators_defaults() {
        let announcement = sign(
            ANNOUNCEMENT,
            &[
                &["d", "r"],
                &["blossoms", "https://blossom.test/", "http://insecure.test"],
            ],
        );
        let state = sign(
            STATE,
            &[
                &["d", "r"],
                &["arweave", A, "tx1", "https://own-gateway.test", "ftp://x"],
                &["blossom", A, BLOB],
                &["arweave", B, "tx2"],
                &["arweave", "short", "tx3"],
            ],
        );
        let defaults = Defaults {
            arweave_gateways: vec![
                "https://arweave.test".into(),
                "https://own-gateway.test".into(),
                "http://insecure.test".into(),
            ],
            blob_servers: vec!["https://default-blossom.test".into()],
        };
        let found = candidates(&announcement, &state, &defaults);
        assert_eq!(
            found[A],
            [
                "https://own-gateway.test/tx1".to_owned(),
                format!("https://blossom.test/{BLOB}"),
                "https://arweave.test/tx1".to_owned(),
                format!("https://default-blossom.test/{BLOB}"),
            ]
        );
        assert_eq!(
            found[B],
            ["https://arweave.test/tx2", "https://own-gateway.test/tx2"]
        );
        assert_eq!(found.len(), 2, "a malformed tag names no object");
    }

    #[test]
    fn a_ref_may_hold_slashes_and_a_commit_id_is_a_ref() {
        let state = sign(
            STATE,
            &[
                &["d", "r"],
                &["refs/heads/main", A],
                &["refs/heads/feature/x", B],
            ],
        );
        let path = |parts: &[&str]| parts.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>();
        let segments = path(&["feature", "x", "src", "lib.rs"]);
        let (name, commit, rest) = resolve(&state, &segments).unwrap();
        assert_eq!(
            (name.as_str(), commit.as_str(), rest),
            ("feature/x", B, &segments[2..])
        );
        let segments = path(&["main"]);
        assert_eq!(resolve(&state, &segments).unwrap().1, A);
        let segments = path(&[B, "a"]);
        let (name, commit, rest) = resolve(&state, &segments).unwrap();
        assert_eq!((name.as_str(), commit.as_str(), rest.len()), (B, B, 1));
        assert!(resolve(&state, &path(&["nope", "a"])).is_none());
    }
}
