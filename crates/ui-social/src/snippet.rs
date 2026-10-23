//! Code snippets (NIP-C0, kind 1337): a snippet's page, `/social/s/<id>`
//! (an id in hex, `note` or `nevent`), and the snippet card the repo page's
//! Snippets tab and an article that embeds a snippet draw.
//!
//! The code is text: escaped, in a monospace block, never run. It is not
//! syntax-highlighted. `/repo` highlights a file with `ui-highlight-web`, a
//! WebAssembly module, and `/social` pages run no script at all; colouring
//! on the server would put syntect in the relay and need a `style`
//! attribute per run, which the pages' policy refuses.
//!
//! A snippet's `repo` tag (NIP-C0) is a URL or a repo's `30617:<pubkey>:<d>`
//! address; this page also reads an `naddr`, a `nostr://<npub>/…/<d>` URL
//! and an `a` tag. A repo on this relay unfurls into its card.

use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use nostr::event::{Event, Kind};
use nostr::filter::Filter;
use nostr::key::PublicKey;
use relay_ui::chrome::{Profiles, ago, escape, person};
use ui_nip34::repo::{RepoAddress, first_tag, tag_values};

use crate::kinds::{REACTION, SNIPPET};
use crate::post::{self, View};
use crate::{PREFIX, Pages, event_by_id, html, load_cards, not_found, note, parse_event_id};

/// The most reactions the page reads.
const LIMIT: usize = 2_000;

/// A snippet's page.
pub(crate) fn path(event: &Event) -> String {
    format!("{PREFIX}/s/{}", event.id.to_hex())
}

/// Where a snippet's code comes from, as its tags say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Origin {
    /// A repo announced on Nostr.
    Repo(RepoAddress),
    /// A web URL (`http` or `https`).
    Url(String),
}

/// The origin a snippet names: its `repo` tag read in each spelling, else
/// its first `a` tag naming a repo. A pure function.
pub(crate) fn origin(snippet: &Event) -> Option<Origin> {
    if let Some(value) = tag_values(snippet, "repo").first().map(|v| v.trim()) {
        let repo = RepoAddress::parse(value)
            .or_else(|| RepoAddress::from_naddr(value))
            .or_else(|| crate::ship::source_repo(value));
        if let Some(repo) = repo {
            return Some(Origin::Repo(repo));
        }
        if value.starts_with("https://") || value.starts_with("http://") {
            return Some(Origin::Url(value.to_owned()));
        }
    }
    RepoAddress::tagged(snippet, "a")
        .into_iter()
        .next()
        .map(Origin::Repo)
}

/// The snippet's name: its `name` tag, else `snippet`.
fn title(snippet: &Event) -> &str {
    first_tag(snippet, "name")
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or("snippet")
}

/// The chips a snippet's tags give: language, extension, runtime, licences.
fn chips(snippet: &Event) -> String {
    let mut out = String::new();
    if let Some(language) = first_tag(snippet, "l") {
        out.push_str(&format!(
            "<span class=\"chip chip-topic\">{}</span>",
            escape(language)
        ));
    }
    if let Some(extension) = first_tag(snippet, "extension") {
        out.push_str(&format!(
            "<span class=\"chip mono\">.{}</span>",
            escape(extension.trim_start_matches('.'))
        ));
    }
    if let Some(runtime) = first_tag(snippet, "runtime") {
        out.push_str(&format!("<span class=\"chip\">{}</span>", escape(runtime)));
    }
    for tag in snippet.tags.iter().map(|tag| tag.as_slice()) {
        if let [tag_name, license, ..] = tag
            && tag_name == "license"
        {
            out.push_str(&format!("<span class=\"chip\">{}</span>", escape(license)));
        }
    }
    out
}

/// A snippet as a card: its name (linking to its page), language, who and
/// when, its description and its code.
pub(crate) fn card(view: &View<'_>, snippet: &Event) -> String {
    let description = first_tag(snippet, "description")
        .map(|text| format!("<p class=\"snippet-desc muted small\">{}</p>", escape(text)))
        .unwrap_or_default();
    let language = first_tag(snippet, "l")
        .map(|language| format!("<span class=\"chip\">{}</span>", escape(language)))
        .unwrap_or_default();
    format!(
        "<article class=\"box snippet\" data-id=\"{}\"><div class=\"box-header\">\
         <a class=\"strong mono\" href=\"{}\">{}</a>{language}<span class=\"muted small\">by {} &middot; {}</span></div>\
         {description}<pre><code>{}</code></pre></article>",
        snippet.id.to_hex(),
        escape(&path(snippet)),
        escape(title(snippet)),
        person(view.links.apps(), view.names, &snippet.pubkey),
        ago(snippet.created_at),
        escape(&snippet.content),
    )
}

pub(crate) async fn page(State(pages): State<Pages>, Path(target): Path<String>) -> Response {
    let Some(id) = parse_event_id(&target) else {
        return not_found(&pages);
    };
    let store = &pages.store;
    let Some(snippet) = event_by_id(store, id)
        .await
        .filter(|event| event.kind.as_u16() == SNIPPET)
    else {
        return not_found(&pages);
    };
    let origin = origin(&snippet);
    let repos: Vec<RepoAddress> = match &origin {
        Some(Origin::Repo(repo)) => vec![repo.clone()],
        _ => Vec::new(),
    };
    let cards = load_cards(store, repos.clone()).await;
    let replies = note::replies(store, &snippet).await;
    let reactions: Vec<Event> = store
        .query(
            Filter::new()
                .kind(Kind::from(REACTION))
                .event(snippet.id)
                .limit(LIMIT),
        )
        .await
        .into_iter()
        .filter(|reaction| note::answers(reaction, &snippet.id.to_hex()))
        .collect();
    let mut keys: Vec<PublicKey> = vec![snippet.pubkey];
    for reply in &replies {
        if !keys.contains(&reply.pubkey) {
            keys.push(reply.pubkey);
        }
    }
    let profiles = Profiles::load(store, &keys).await;
    let links = pages.links();
    let no_demos = HashMap::new();
    let view = View {
        links: &links,
        names: profiles.names(),
        cards: &cards,
        demos: &no_demos,
        gateway: pages.gateway(),
    };

    let lines = snippet.content.lines().count();
    let mut main = format!(
        "<p><a class=\"muted small\" href=\"{PREFIX}\">&larr; Back to the feed</a></p>\
         <article class=\"post\" data-kind=\"1337\" data-id=\"{}\">{}\
         <h1 class=\"page-title mono\">{}</h1><div class=\"chips\">{}</div>",
        snippet.id.to_hex(),
        post::by_line(&view, &snippet, "<span class=\"chip\">snippet</span>", ""),
        escape(title(&snippet)),
        chips(&snippet),
    );
    if let Some(description) = first_tag(&snippet, "description") {
        main.push_str(&format!("<p class=\"text\">{}</p>", escape(description)));
    }
    main.push_str(&format!(
        "<div class=\"box snippet\"><div class=\"box-header\"><span class=\"strong mono\">{}</span>\
         <span class=\"muted small\">{lines} {}</span></div><pre class=\"snippet-code\"><code>{}</code></pre></div>",
        escape(title(&snippet)),
        ui_nip34::card::plural(lines, "line", "lines"),
        escape(&snippet.content),
    ));
    let deps: Vec<&str> = snippet
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .filter_map(|tag| match tag {
            [tag_name, dep, ..] if tag_name == "dep" => Some(dep.as_str()),
            _ => None,
        })
        .collect();
    if !deps.is_empty() {
        main.push_str("<h2 class=\"post-title\">Dependencies</h2><ul class=\"deps\">");
        for dep in deps {
            main.push_str(&format!("<li><code>{}</code></li>", escape(dep)));
        }
        main.push_str("</ul>");
    }
    match &origin {
        Some(Origin::Repo(repo)) => {
            if cards.contains_key(repo) {
                main.push_str(&post::cards_html(&view, std::slice::from_ref(repo)));
            } else {
                main.push_str(&format!(
                    "<p class=\"muted small\">From the repository <code>{}</code>, which is not on this relay.</p>",
                    escape(&repo.coordinate())
                ));
            }
        }
        Some(Origin::Url(url)) => main.push_str(&format!(
            "<p class=\"muted small\">From <a href=\"{0}\" rel=\"nofollow ugc noopener noreferrer\">{0}</a></p>",
            escape(url)
        )),
        None => {}
    }
    main.push_str(&note::tally_chips(&reactions));
    main.push_str(
        "<p class=\"muted small\">A code snippet (NIP-C0, kind 1337), shown as text: \
         this page runs nothing and colours nothing. Copy it from here or open it in a client.</p></article>",
    );
    let body = format!(
        "<div class=\"split\"><div class=\"main\">{main}</div>{}</div>",
        note::rail(&view, &replies, repos.first())
    );
    html(
        &pages,
        crate::Nav::Other,
        StatusCode::OK,
        title(&snippet),
        None,
        &body,
    )
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;
    use relay_ui::chrome::npub;

    use super::*;

    fn snippet(tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(SNIPPET), "fn main() {}")
            .tags(tags.iter().map(|parts| Tag::parse(parts.to_vec()).unwrap()))
            .finalize(&Keys::generate())
            .unwrap()
    }

    #[test]
    fn a_snippet_names_its_origin_in_any_spelling() {
        let owner = Keys::generate().public_key();
        let repo = RepoAddress::new(owner, "tidepool").unwrap();
        let coordinate = repo.coordinate();
        let url = format!("nostr://{}/relay.test/tidepool", npub(&owner));
        for tags in [
            vec![["repo", coordinate.as_str()]],
            vec![["repo", url.as_str()]],
            vec![["a", coordinate.as_str()]],
        ] {
            let tags: Vec<&[&str]> = tags.iter().map(|tag| tag.as_slice()).collect();
            assert_eq!(origin(&snippet(&tags)), Some(Origin::Repo(repo.clone())));
        }
        assert_eq!(
            origin(&snippet(&[&["repo", "https://github.com/x/y"]])),
            Some(Origin::Url("https://github.com/x/y".to_owned()))
        );
        assert_eq!(origin(&snippet(&[&["repo", "javascript:alert(1)"]])), None);
        assert_eq!(origin(&snippet(&[])), None);
    }

    #[test]
    fn a_snippets_tags_are_escaped_chips() {
        let event = snippet(&[
            &["l", "rust"],
            &["extension", ".rs"],
            &["license", "MIT"],
            &["license", "<b>"],
        ]);
        let chips = chips(&event);
        assert!(chips.contains(">rust<"));
        assert!(chips.contains(">.rs<"));
        assert!(chips.contains(">MIT<"));
        assert!(chips.contains("&lt;b&gt;"));
        assert_eq!(title(&event), "snippet");
    }
}
