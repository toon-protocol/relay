//! How one post is drawn, wherever it appears: the by-line, a note with its
//! repo cards, a ship post as a launch, a repo boost, an article card, a
//! merge, a new repo. The feed, the repo page and the post page all draw
//! posts with these.

use std::collections::HashMap;

use nostr::event::Event;
use nostr::key::PublicKey;
use relay_ui::chrome::{ago, avatar, escape, icon, name, plain_name, svg};
use ui_nip34::card::{Links, RepoCard};
use ui_nip34::repo::{RepoAddress, first_tag};

use crate::PREFIX;
use crate::content::{self, first_line};
use crate::demo::Demo;
use crate::kinds::{self, ANNOUNCEMENT, APP_HANDLER, ARTICLE, HIGHLIGHT, THREAD};
use crate::media::{self, Media};
use crate::ship;

/// The most characters of a note a list shows before it links to the rest.
const CLIP: usize = 1_000;

/// What drawing a post needs beyond the post.
pub(crate) struct View<'a> {
    pub(crate) links: &'a Links,
    pub(crate) names: &'a HashMap<PublicKey, String>,
    pub(crate) cards: &'a HashMap<RepoAddress, RepoCard>,
    /// The live demo of each repo drawn, where there is one
    /// ([`crate::demo`]).
    pub(crate) demos: &'a HashMap<RepoAddress, Demo>,
    pub(crate) gateway: Option<&'a str>,
}

impl View<'_> {
    /// Whether `key` is an authorized author of `repo` (ADR 0002), as far as
    /// the loaded cards say.
    pub(crate) fn is_maker(&self, repo: &RepoAddress, key: &PublicKey) -> bool {
        self.cards.get(repo).is_some_and(|card| card.is_maker(key))
    }

    /// The repo `event` launches, if it is a ship post.
    pub(crate) fn ship(&self, event: &Event) -> Option<RepoAddress> {
        ship::ship_repo(event, |repo, key| self.is_maker(repo, key))
    }

    /// The live demo's URL for `repo`, if it has one and a gateway is set.
    pub(crate) fn demo_url(&self, repo: &RepoAddress) -> Option<String> {
        self.demos.get(repo)?.url(self.gateway)
    }

    /// `card`, with its live demo when it has one.
    pub(crate) fn card(&self, card: &RepoCard) -> String {
        card.render_with_demo(self.links, self.demo_url(card.address()).as_deref())
    }
}

/// The page of one post.
pub(crate) fn event_path(event: &Event) -> String {
    format!("{PREFIX}/e/{}", event.id.to_hex())
}

/// The person who wrote `event`, its chips, and when, linking to the post.
pub(crate) fn by_line(view: &View<'_>, event: &Event, chips: &str, context: &str) -> String {
    format!(
        "<div class=\"by\">{}<div class=\"who\"><div class=\"line\">\
         <a href=\"{PREFIX}/following/{}\">{}</a>{chips}</div>\
         <span class=\"muted small\"><a class=\"muted\" href=\"{}\">{}</a>{}</span></div></div>",
        avatar(&plain_name(view.names, &event.pubkey), false),
        relay_ui::chrome::npub(&event.pubkey),
        name(view.names, &event.pubkey),
        escape(&event_path(event)),
        ago(event.created_at),
        if context.is_empty() {
            String::new()
        } else {
            format!(" &middot; {context}")
        },
    )
}

/// The repos a post names: its `a` tags, then the repos its text mentions.
pub(crate) fn repos_of(event: &Event) -> Vec<RepoAddress> {
    let mut repos = RepoAddress::tagged(event, "a");
    for repo in RepoAddress::tagged(event, "A") {
        if !repos.contains(&repo) {
            repos.push(repo);
        }
    }
    if matches!(
        event.kind.as_u16(),
        kinds::NOTE | THREAD | kinds::COMMENT | ARTICLE
    ) {
        for repo in content::render(&event.content, &[], &HashMap::new()).repos {
            if !repos.contains(&repo) {
                repos.push(repo);
            }
        }
    }
    if event.kind.as_u16() == ANNOUNCEMENT
        && let Some(repo) = RepoAddress::of(event)
        && !repos.contains(&repo)
    {
        repos.insert(0, repo);
    }
    repos
}

/// The cards of `repos` that are loaded.
pub(crate) fn cards_html(view: &View<'_>, repos: &[RepoAddress]) -> String {
    repos
        .iter()
        .filter_map(|repo| view.cards.get(repo))
        .map(|card| view.card(card))
        .collect()
}

/// The card of the repo a launch ships, without its own Live demo button:
/// the launch's buttons already lead there.
pub(crate) fn launch_card(view: &View<'_>, repo: &RepoAddress) -> String {
    view.cards
        .get(repo)
        .map(|card| card.render(view.links))
        .unwrap_or_default()
}

/// A note's text, clipped to [`CLIP`] in a list, with the media its `imeta`
/// tags show left out of it.
fn text_html(view: &View<'_>, event: &Event, media: &[Media], clipped: bool) -> String {
    let hidden: Vec<String> = media.iter().map(|m| m.url().to_owned()).collect();
    let text = if clipped {
        content::clip(&event.content, CLIP)
    } else {
        event.content.clone()
    };
    let drawn = content::render(&text, &hidden, view.names);
    if drawn.html.trim().is_empty() {
        String::new()
    } else {
        format!("<p class=\"text\">{}</p>", drawn.html.trim())
    }
}

/// A grid of small media.
fn thumbs(media: &[Media]) -> String {
    if media.is_empty() {
        return String::new();
    }
    let mut out = String::from("<div class=\"thumbs\">");
    for item in media {
        out.push_str(&item.html(""));
    }
    out.push_str("</div>");
    out
}

/// A plain note (or thread, comment, highlight) in a list: who, the text,
/// its media and the cards of the repos it names.
pub(crate) fn note_card(view: &View<'_>, event: &Event) -> String {
    let media = media::imeta(event);
    let chip = match event.kind.as_u16() {
        kinds::NOTE => String::new(),
        kind => format!("<span class=\"chip\">{}</span>", kinds::label(kind)),
    };
    let mut out = format!(
        "<article class=\"post\" data-kind=\"{}\" data-id=\"{}\">{}",
        event.kind.as_u16(),
        event.id.to_hex(),
        by_line(view, event, &chip, "")
    );
    if event.kind.as_u16() == THREAD
        && let Some(title) = first_tag(event, "title")
    {
        out.push_str(&format!(
            "<a class=\"post-title\" href=\"{}\">{}</a>",
            escape(&event_path(event)),
            escape(title)
        ));
    }
    if event.kind.as_u16() == HIGHLIGHT {
        out.push_str(&format!(
            "<blockquote class=\"markdown\"><p>{}</p></blockquote>",
            escape(&content::clip(&event.content, CLIP))
        ));
    } else {
        out.push_str(&text_html(view, event, &media, true));
    }
    out.push_str(&thumbs(&media));
    out.push_str(&cards_html(view, &repos_of(event)));
    out.push_str("</article>");
    out
}

/// The buttons under a launch: the live demo when there is one, the repo
/// on `/repo`, its social page, and the post itself.
pub(crate) fn launch_actions(view: &View<'_>, repo: &RepoAddress, event: &Event) -> String {
    let mut out = String::from("<div class=\"actions-row\">");
    if let Some(url) = view.demo_url(repo) {
        out.push_str(&format!(
            "<a class=\"btn btn-success btn-lg\" href=\"{}\" rel=\"noopener noreferrer\" data-demo>{}Live demo</a>",
            escape(&url),
            svg(icon::PLAY, "")
        ));
    }
    if let Some(path) = view.links.repo_path(repo) {
        out.push_str(&format!(
            "<a class=\"btn btn-lg\" href=\"{}\">{}Open repo</a>",
            escape(&path),
            svg(icon::CODE, "")
        ));
    }
    if let Some(path) = view.links.social_path(repo) {
        out.push_str(&format!(
            "<a class=\"btn btn-lg\" href=\"{}\">{}Repo page</a>",
            escape(&path),
            svg(icon::REPO, "")
        ));
    }
    out.push_str(&format!(
        "<a class=\"btn btn-lg\" href=\"{}\">{}Discuss</a></div>",
        escape(&event_path(event)),
        svg(icon::COMMENT, "")
    ));
    out
}

/// The chips that mark a launch.
pub(crate) fn ship_chips(event: &Event) -> String {
    let mut chips = format!(
        "<span class=\"chip chip-ship\">{}Ship</span>",
        svg(icon::SHIP, "icon-sm")
    );
    if event
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .any(|tag| matches!(tag, [name, value, ..] if name == "t" && value == "shakespeare"))
    {
        chips.push_str("<span class=\"chip\">made with Shakespeare</span>");
    }
    chips
}

/// A ship post's title: an app event's name, else the note's first line.
pub(crate) fn ship_title(event: &Event) -> String {
    if event.kind.as_u16() == APP_HANDLER {
        let (name, _) = ship::app_metadata(event);
        if let Some(name) = name {
            return name;
        }
    }
    content::headline(&event.content, 90)
}

/// The media a launch shows: its `imeta` media, or an app's `image` tag.
pub(crate) fn launch_media(event: &Event) -> Vec<Media> {
    let mut media = media::imeta(event);
    if media.is_empty()
        && let Some(image) = media::image_tag(event)
    {
        media.push(image);
    }
    media
}

/// A ship post in a list: the launch, its hero media, the repo card and the
/// buttons.
pub(crate) fn ship_card(view: &View<'_>, event: &Event, repo: &RepoAddress) -> String {
    let media = launch_media(event);
    let mut out = format!(
        "<article class=\"post\" data-kind=\"{}\" data-id=\"{}\" data-ship>{}",
        event.kind.as_u16(),
        event.id.to_hex(),
        by_line(view, event, &ship_chips(event), "launched a repo they keep")
    );
    if event.kind.as_u16() == APP_HANDLER {
        let (name, about) = ship::app_metadata(event);
        if let Some(name) = name {
            out.push_str(&format!(
                "<a class=\"post-title\" href=\"{}\">{}</a>",
                escape(&event_path(event)),
                escape(&name)
            ));
        }
        if let Some(about) = about {
            out.push_str(&format!("<p class=\"text\">{}</p>", escape(&about)));
        }
    } else {
        out.push_str(&text_html(view, event, &media, true));
    }
    if let Some((hero, rest)) = media.split_first() {
        if hero.is_video() {
            // A video's controls are its own; a link around them would steal
            // the click.
            out.push_str(&hero.html("hero"));
        } else {
            out.push_str(&format!(
                "<a href=\"{}\" aria-label=\"Open the launch\">{}</a>",
                escape(&event_path(event)),
                hero.html("hero")
            ));
        }
        out.push_str(&thumbs(rest));
    }
    out.push_str(&launch_card(view, repo));
    out.push_str(&launch_actions(view, repo, event));
    out.push_str("</article>");
    out
}

/// A generic repost (kind 16) of a repo: who boosted it, and its card.
pub(crate) fn repost_card(view: &View<'_>, event: &Event) -> Option<String> {
    let repo = RepoAddress::tagged(event, "a").into_iter().next()?;
    let card = view.cards.get(&repo)?;
    Some(format!(
        "<article class=\"post\" data-kind=\"16\" data-id=\"{}\"><p class=\"context\">{}{} boosted a repo &middot; {}</p>{}</article>",
        event.id.to_hex(),
        svg(icon::REPOST, ""),
        name(view.names, &event.pubkey),
        ago(event.created_at),
        view.card(card),
    ))
}

/// A long-form article (kind 30023) as a card: its image, title, summary.
pub(crate) fn article_card(view: &View<'_>, event: &Event) -> String {
    let title = first_tag(event, "title")
        .filter(|title| !title.trim().is_empty())
        .map(str::to_owned)
        .unwrap_or_else(|| first_line(&event.content, 120));
    let summary = first_tag(event, "summary")
        .map(|summary| content::clip(summary, 280))
        .unwrap_or_else(|| content::clip(&event.content, 280));
    let repo_line = repos_of(event)
        .into_iter()
        .filter_map(|repo| view.cards.get(&repo))
        .map(|card| {
            format!(
                "<span class=\"chip\">{}{}</span>",
                svg(icon::REPO, "icon-sm"),
                escape(&card.title())
            )
        })
        .collect::<String>();
    format!(
        "<article class=\"post\" data-kind=\"30023\" data-id=\"{}\">{}{}\
         <a class=\"post-title\" href=\"{}\">{}</a><p class=\"muted\">{}</p>\
         <div class=\"chips\"><span class=\"chip\">article</span>{repo_line}</div></article>",
        event.id.to_hex(),
        by_line(view, event, "", "article"),
        media::image_tag(event)
            .map(|image| thumbs(&[image]))
            .unwrap_or_default(),
        escape(&event_path(event)),
        escape(&title),
        escape(&summary),
    )
}

/// A merge (kind 1631) by a repo's maker or the change's own author, of an
/// issue, patch or pull request in `targets`.
pub(crate) fn merged_row(
    view: &View<'_>,
    status: &Event,
    targets: &HashMap<String, Event>,
) -> Option<String> {
    let repo = RepoAddress::tagged(status, "a").into_iter().next()?;
    let card = view.cards.get(&repo)?;
    let target = status
        .tags
        .iter()
        .map(|tag| tag.as_slice())
        .find_map(|tag| match tag {
            [name, id, ..] if name == "e" => targets.get(id),
            _ => None,
        })?;
    if !(card.is_maker(&status.pubkey) || status.pubkey == target.pubkey) {
        return None;
    }
    let (verb, chip) = if target.kind.as_u16() == kinds::ISSUE {
        ("resolved issue", "Resolved")
    } else {
        ("merged", "Merged")
    };
    let title = first_tag(target, "subject")
        .map(str::to_owned)
        .unwrap_or_else(|| first_line(&target.content, 80));
    let section = if target.kind.as_u16() == kinds::ISSUE {
        "issues"
    } else {
        "pulls"
    };
    let link = view
        .links
        .repo_path(&repo)
        .map(|base| format!("{base}/{section}/{}", target.id.to_hex()));
    let title_html = match link {
        Some(link) => format!("<a href=\"{}\">{}</a>", escape(&link), escape(&title)),
        None => escape(&title),
    };
    Some(format!(
        "<article class=\"post post-slim\" data-kind=\"1631\" data-id=\"{}\">\
         <span class=\"chip chip-merged\">{}{chip}</span><p><span class=\"strong\">{}</span> {} {verb} {title_html} by {}</p>\
         <span class=\"muted small\">{}</span></article>",
        status.id.to_hex(),
        svg(icon::MERGED, "icon-sm"),
        escape(&card.title()),
        name(view.names, &status.pubkey),
        name(view.names, &target.pubkey),
        ago(status.created_at),
    ))
}

/// A new issue or pull request on a followed repo.
pub(crate) fn thread_row(view: &View<'_>, event: &Event) -> Option<String> {
    let repo = RepoAddress::tagged(event, "a").into_iter().next()?;
    let card = view.cards.get(&repo)?;
    let (noun, section, glyph) = if event.kind.as_u16() == kinds::ISSUE {
        ("an issue", "issues", icon::ISSUE)
    } else {
        ("a pull request", "pulls", icon::PULL)
    };
    let title = first_tag(event, "subject")
        .map(str::to_owned)
        .unwrap_or_else(|| first_line(&event.content, 80));
    let title_html = match view.links.repo_path(&repo) {
        Some(base) => format!(
            "<a href=\"{}\">{}</a>",
            escape(&format!("{base}/{section}/{}", event.id.to_hex())),
            escape(&title)
        ),
        None => escape(&title),
    };
    Some(format!(
        "<article class=\"post post-slim\" data-kind=\"{}\" data-id=\"{}\">{}<p>{} opened {noun} on <span class=\"strong\">{}</span>: {title_html}</p>\
         <span class=\"muted small\">{}</span></article>",
        event.kind.as_u16(),
        event.id.to_hex(),
        svg(glyph, "open"),
        name(view.names, &event.pubkey),
        escape(&card.title()),
        ago(event.created_at),
    ))
}

/// A new repository announcement: who published it, and its card.
pub(crate) fn announced_card(view: &View<'_>, event: &Event) -> Option<String> {
    let repo = RepoAddress::of(event)?;
    let card = view.cards.get(&repo)?;
    // Only the announcement the card is built from: an older one replaced by
    // an edit is not news.
    (card.announcement().id == event.id).then_some(())?;
    Some(format!(
        "<article class=\"post\" data-kind=\"30617\" data-id=\"{}\"><p class=\"context\">{}{} published a repository &middot; {}</p>{}</article>",
        event.id.to_hex(),
        svg(icon::REPO, ""),
        name(view.names, &event.pubkey),
        ago(event.created_at),
        view.card(card),
    ))
}

/// A post in a repo page's list: who (marked when it is the maker), what
/// kind it is, and its text, title or summary.
pub(crate) fn repo_post(view: &View<'_>, event: &Event, maker: bool) -> String {
    let mut chips = String::new();
    if maker {
        chips.push_str("<span class=\"chip chip-maker\">Maker</span>");
    }
    if view.ship(event).is_some() {
        chips.push_str(&ship_chips(event));
    } else {
        chips.push_str(&format!(
            "<span class=\"chip\">{}</span>",
            kinds::label(event.kind.as_u16())
        ));
    }
    let mut out = format!(
        "<article class=\"post\" data-kind=\"{}\" data-id=\"{}\">{}",
        event.kind.as_u16(),
        event.id.to_hex(),
        by_line(view, event, &chips, "")
    );
    match event.kind.as_u16() {
        ARTICLE | APP_HANDLER => {
            let title = if event.kind.as_u16() == ARTICLE {
                first_tag(event, "title")
                    .map(str::to_owned)
                    .unwrap_or_else(|| first_line(&event.content, 120))
            } else {
                ship_title(event)
            };
            out.push_str(&format!(
                "<a class=\"post-title\" href=\"{}\">{}</a>",
                escape(&event_path(event)),
                escape(&title)
            ));
            if let Some(summary) = first_tag(event, "summary") {
                out.push_str(&format!(
                    "<p class=\"muted\">{}</p>",
                    escape(&content::clip(summary, 280))
                ));
            }
        }
        HIGHLIGHT => out.push_str(&format!(
            "<blockquote class=\"markdown\"><p>{}</p></blockquote>",
            escape(&content::clip(&event.content, CLIP))
        )),
        _ => {
            if let Some(title) = first_tag(event, "title") {
                out.push_str(&format!(
                    "<a class=\"post-title\" href=\"{}\">{}</a>",
                    escape(&event_path(event)),
                    escape(title)
                ));
            }
            let media = media::imeta(event);
            out.push_str(&text_html(view, event, &media, true));
            out.push_str(&thumbs(&media));
        }
    }
    out.push_str("</article>");
    out
}
