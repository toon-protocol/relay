//! One post's page, `/social/e/<id>` (an id in hex, `note` or `nevent`):
//! the post, the repo cards it unfurls and its replies. A ship post is drawn
//! as a launch (proposal 3): its hero media, the repo card, buttons that
//! are links, reactions counted by person, and a comment rail with the
//! maker's replies marked.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, RawQuery, State};
use axum::http::StatusCode;
use axum::response::Response;
use nostr::event::{Event, Kind};
use nostr::filter::{Filter, SingleLetterTag};
use nostr::key::PublicKey;
use relay_ui::chrome::{Profiles, ago, avatar, escape, icon, name, plain_name, svg};
use ui_nip34::card::plural;
use ui_nip34::repo::{RepoAddress, first_tag};

use crate::kinds::{APP_HANDLER, ARTICLE, COMMENT, NOTE, REACTION};
use crate::post::{self, View};
use crate::ship::{self, Site};
use crate::{PREFIX, Pages, content, event_by_id, html, load_cards, not_found, parse_event_id};

/// The most replies or reactions the page reads.
const LIMIT: usize = 2_000;

/// The replies to `event`: kind 1 notes whose `e` tags name it (not as a
/// mention) and NIP-22 comments whose root `E` names it, oldest first.
async fn replies(store: &relay_ui::ReadOnlyStore, event: &Event) -> Vec<Event> {
    let id = event.id.to_hex();
    let notes = store
        .query(
            Filter::new()
                .kind(Kind::from(NOTE))
                .event(event.id)
                .limit(LIMIT),
        )
        .await;
    let comments = store
        .query(
            Filter::new()
                .kind(Kind::from(COMMENT))
                .custom_tag(SingleLetterTag::UPPERCASE_E, id.clone())
                .limit(LIMIT),
        )
        .await;
    let mut found: Vec<Event> = notes
        .into_iter()
        .filter(|note| answers(note, &id))
        .chain(comments.into_iter().filter(|comment| {
            comment
                .tags
                .iter()
                .map(|tag| tag.as_slice())
                .any(|tag| matches!(tag, [name, value, ..] if name == "E" && *value == id))
        }))
        .collect();
    found.sort_by_key(|event| (event.created_at, event.id));
    found.dedup_by_key(|event| event.id);
    found
}

/// Whether a note's `e` tags name `id` other than as a mention.
fn answers(note: &Event, id: &str) -> bool {
    note.tags
        .iter()
        .map(|tag| tag.as_slice())
        .any(|tag| match tag {
            [name, value] | [name, value, _] => name == "e" && value == id,
            [name, value, _, marker, ..] => name == "e" && value == id && marker != "mention",
            _ => false,
        })
}

/// Reactions by content, each counting the distinct people who reacted so,
/// most first: `+` (or empty) is a like, `-` a dislike, else the emoji or
/// shortcode as written. A pure function.
pub(crate) fn tally(reactions: &[Event]) -> Vec<(String, usize)> {
    let mut people: HashMap<String, HashSet<PublicKey>> = HashMap::new();
    for reaction in reactions {
        let content = reaction.content.trim();
        let key = if content.is_empty() { "+" } else { content };
        people
            .entry(content::clip(key, 16))
            .or_default()
            .insert(reaction.pubkey);
    }
    let mut ranked: Vec<(String, usize)> = people
        .into_iter()
        .map(|(content, keys)| (content, keys.len()))
        .collect();
    ranked.sort_by(|(a, x), (b, y)| y.cmp(x).then_with(|| a.cmp(b)));
    ranked
}

pub(crate) async fn page(
    State(pages): State<Pages>,
    Path(target): Path<String>,
    RawQuery(query): RawQuery,
) -> Response {
    let Some(id) = parse_event_id(&target) else {
        return not_found(&pages);
    };
    let store = &pages.store;
    let Some(event) = event_by_id(store, id).await else {
        return not_found(&pages);
    };
    let repos = post::repos_of(&event);
    let cards = load_cards(store, repos.clone()).await;
    let replies = replies(store, &event).await;
    let reactions: Vec<Event> = store
        .query(
            Filter::new()
                .kind(Kind::from(REACTION))
                .event(event.id)
                .limit(LIMIT),
        )
        .await
        .into_iter()
        .filter(|reaction| answers(reaction, &event.id.to_hex()))
        .collect();
    let mut keys: Vec<PublicKey> = vec![event.pubkey];
    for key in replies
        .iter()
        .map(|reply| reply.pubkey)
        .chain(content::mentioned_keys(&event.content))
        .chain(
            replies
                .iter()
                .flat_map(|reply| content::mentioned_keys(&reply.content)),
        )
    {
        if !keys.contains(&key) {
            keys.push(key);
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
    let ship = view.ship(&event);
    let mut demos: HashMap<RepoAddress, Site> = HashMap::new();
    if let Some(repo) = &ship
        && pages.gateway().is_some()
    {
        let site = match ship::app_site(&event) {
            Some(site) => Some(site),
            None => match cards.get(repo) {
                Some(card) => ship::demo(store, card).await,
                None => None,
            },
        };
        if let Some(site) = site {
            demos.insert(repo.clone(), site);
        }
    }
    let view = View {
        demos: &demos,
        ..view
    };
    let main = match &ship {
        Some(repo) => launch(&view, &event, repo, &reactions),
        None => plain(&view, &event, &repos, &reactions),
    };
    let maker_of = ship.as_ref().or(repos.first());
    let mut body = crate::reports::post_banner(
        store,
        &event,
        ship.as_ref().and_then(|repo| cards.get(repo)),
        &pages.links(),
        query.as_deref(),
    )
    .await;
    body.push_str(&format!(
        "<div class=\"split\"><div class=\"main\">{main}</div>{}</div>",
        rail(&view, &replies, maker_of)
    ));
    let title = match &ship {
        Some(_) => post::ship_title(&event),
        None => first_tag(&event, "title")
            .map(str::to_owned)
            .unwrap_or_else(|| content::first_line(&event.content, 80)),
    };
    html(
        &pages,
        StatusCode::OK,
        if title.is_empty() { "Post" } else { &title },
        None,
        &body,
    )
}

/// Reactions as chips, counted by person.
fn tally_chips(reactions: &[Event]) -> String {
    let ranked = tally(reactions);
    if ranked.is_empty() {
        return String::new();
    }
    let mut out = String::from("<div class=\"chips\" aria-label=\"Reactions\">");
    for (content, people) in ranked.into_iter().take(6) {
        out.push_str(&format!(
            "<span class=\"chip\" title=\"{people} {}\">{} {people}</span>",
            plural(people, "person", "people"),
            escape(&content)
        ));
    }
    out.push_str("</div>");
    out
}

/// A ship post, as a launch.
fn launch(view: &View<'_>, event: &Event, repo: &RepoAddress, reactions: &[Event]) -> String {
    let media = post::launch_media(event);
    let role = if event.pubkey == *repo.owner() {
        "owns this repo"
    } else {
        "maintains this repo"
    };
    let mut out = format!(
        "<p><a class=\"muted small\" href=\"{PREFIX}\">&larr; Back to the feed</a></p>\
         <div class=\"chips\">{}<span class=\"chip\">launched {}</span></div>\
         <h1 class=\"launch-title\">{}</h1>\
         <div class=\"post\"><div class=\"by\">{}<div class=\"who\"><div class=\"line\">{}\
         <span class=\"chip chip-maker\">Maker</span></div><span class=\"muted small\">{role}</span></div></div></div>",
        post::ship_chips(event),
        ago(event.created_at),
        escape(&post::ship_title(event)),
        avatar(&plain_name(view.names, &event.pubkey), false),
        name(view.names, &event.pubkey),
    );
    if let Some((hero, rest)) = media.split_first() {
        out.push_str(&hero.html("hero"));
        if !rest.is_empty() {
            out.push_str("<div class=\"thumbs\">");
            for item in rest {
                out.push_str(&item.html(""));
            }
            out.push_str("</div>");
        }
    }
    out.push_str(&post::launch_actions(view, repo, event));
    if let Some(card) = view.cards.get(repo) {
        out.push_str(&format!(
            "<p class=\"muted small\">{}{} {} starred {}.</p>",
            svg(icon::STAR, "icon-sm"),
            card.stars(),
            plural(card.stars(), "person", "people"),
            escape(&card.title())
        ));
    }
    if event.kind.as_u16() == APP_HANDLER {
        if let (_, Some(about)) = ship::app_metadata(event) {
            out.push_str(&format!("<p class=\"text\">{}</p>", escape(&about)));
        }
    } else {
        let hidden: Vec<String> = media.iter().map(|m| m.url().to_owned()).collect();
        let text = content::render(&event.content, &hidden, view.names);
        out.push_str(&format!("<p class=\"text\">{}</p>", text.html.trim()));
    }
    out.push_str(&post::cards_html(view, std::slice::from_ref(repo)));
    out.push_str(&tally_chips(reactions));
    out
}

/// Any other post: who, the text (an article's Markdown), its media and the
/// cards of the repos it names.
fn plain(view: &View<'_>, event: &Event, repos: &[RepoAddress], reactions: &[Event]) -> String {
    let mut out = format!(
        "<p><a class=\"muted small\" href=\"{PREFIX}\">&larr; Back to the feed</a></p>\
         <article class=\"post\" data-kind=\"{}\">{}",
        event.kind.as_u16(),
        post::by_line(
            view,
            event,
            &format!(
                "<span class=\"chip\">{}</span>",
                crate::kinds::label(event.kind.as_u16())
            ),
            ""
        )
    );
    if event.kind.as_u16() == ARTICLE {
        if let Some(title) = first_tag(event, "title") {
            out.push_str(&format!("<h1 class=\"page-title\">{}</h1>", escape(title)));
        }
        if let Some(image) = crate::media::image_tag(event) {
            out.push_str(&image.html("hero"));
        }
        out.push_str(&format!(
            "<div class=\"markdown\">{}</div>",
            ui_markdown::to_html(&ui_markdown::parse(
                &event.content,
                ui_markdown::absolute_only
            ))
        ));
    } else {
        let media = crate::media::imeta(event);
        let hidden: Vec<String> = media.iter().map(|m| m.url().to_owned()).collect();
        if let Some(title) = first_tag(event, "title") {
            out.push_str(&format!("<h2 class=\"post-title\">{}</h2>", escape(title)));
        }
        let text = content::render(&event.content, &hidden, view.names);
        out.push_str(&format!("<p class=\"text\">{}</p>", text.html.trim()));
        if !media.is_empty() {
            out.push_str("<div class=\"thumbs\">");
            for item in &media {
                out.push_str(&item.html(""));
            }
            out.push_str("</div>");
        }
    }
    out.push_str(&post::cards_html(view, repos));
    out.push_str(&tally_chips(reactions));
    out.push_str("</article>");
    out
}

/// The comment rail: the replies, the maker's marked.
fn rail(view: &View<'_>, replies: &[Event], repo: Option<&RepoAddress>) -> String {
    let mut out = format!(
        "<aside class=\"sidebar\"><div><h2>{} {}</h2><p class=\"muted small\">Replies to the post \
         (NIP-10) and comments on it (NIP-22). They are written in a client; each is a paid write.</p></div>\
         <div class=\"replies\">",
        replies.len(),
        plural(replies.len(), "reply", "replies"),
    );
    for reply in replies {
        let maker = repo.is_some_and(|repo| view.is_maker(repo, &reply.pubkey));
        let text = content::render(&reply.content, &[], view.names);
        out.push_str(&format!(
            "<article class=\"reply\" data-id=\"{}\">{}<div class=\"body\"><div class=\"line\">{}{}\
             <a class=\"muted small\" href=\"{}\">{}</a></div><p class=\"text\">{}</p></div></article>",
            reply.id.to_hex(),
            avatar(&plain_name(view.names, &reply.pubkey), true),
            name(view.names, &reply.pubkey),
            if maker {
                " <span class=\"chip chip-maker\">Maker</span> "
            } else {
                " "
            },
            escape(&post::event_path(reply)),
            ago(reply.created_at),
            text.html.trim(),
        ));
    }
    out.push_str("</div></aside>");
    out
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;

    use super::*;

    fn reaction(keys: &Keys, content: &str) -> Event {
        EventBuilder::new(Kind::from(REACTION), content)
            .tags([Tag::parse(["e", &"a".repeat(64)]).unwrap()])
            .finalize(keys)
            .unwrap()
    }

    #[test]
    fn reactions_are_counted_by_person_and_an_empty_one_is_a_like() {
        let (a, b) = (Keys::generate(), Keys::generate());
        let reactions = [
            reaction(&a, "+"),
            reaction(&a, "+"),
            reaction(&b, ""),
            reaction(&b, ":shipit:"),
        ];
        assert_eq!(
            tally(&reactions),
            vec![("+".to_owned(), 2), (":shipit:".to_owned(), 1)]
        );
    }

    #[test]
    fn a_mention_is_not_an_answer() {
        let keys = Keys::generate();
        let id = "b".repeat(64);
        let note = |tags: Vec<Tag>| {
            EventBuilder::new(Kind::TextNote, "")
                .tags(tags)
                .finalize(&keys)
                .unwrap()
        };
        assert!(answers(
            &note(vec![Tag::parse(["e", id.as_str()]).unwrap()]),
            &id
        ));
        assert!(!answers(
            &note(vec![Tag::parse(["e", id.as_str(), "", "mention"]).unwrap()]),
            &id
        ));
        assert!(!answers(&note(vec![]), &id));
    }
}
