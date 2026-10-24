//! The pieces `/repo` pages are drawn from, in rig-web's look: the page
//! shell, the repo header and its tabs, and the branch menu. What `/repo`
//! shares with the other apps (the shell's top bar, icons, names, avatars,
//! dates) is `relay_ui::chrome`; this module only adds what is `/repo`'s own.

use std::collections::HashMap;

use nostr::event::Event;
use nostr::key::PublicKey;
use relay_ui::chrome::{self, App, Apps};
use relay_ui::meta::Preview;

pub(crate) use relay_ui::chrome::{ago, avatar, icon, name, plain_name, svg};

use crate::{escape, first_tag, repo_name, repo_path};

/// The whole page: the shared shell, with `/repo`'s bar and `body`.
pub(crate) fn page(apps: &Apps, title: &str, body: &str) -> String {
    chrome::page(apps, App::Repo, title, "", body)
}

/// [`page`], with what it tells a link unfurler.
pub(crate) fn page_with(apps: &Apps, title: &str, body: &str, preview: &Preview) -> String {
    chrome::page_with(apps, App::Repo, title, "", body, preview)
}

/// The tab a repo page is under.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Code,
    Issues,
    Pulls,
    Commits,
}

/// The repo header: its glyph, `owner / name`, the visibility pill, the
/// description and the tabs, with `tab` current. When `/social` is on, a
/// last tab leads to the repo's social page there.
pub(crate) fn repo_header(
    apps: &Apps,
    owner: &PublicKey,
    d: &str,
    announcement: &Event,
    names: &HashMap<PublicKey, String>,
    tab: Tab,
) -> String {
    let base = repo_path(owner, d);
    let mut out = chrome::repo_title(&name(names, owner), repo_name(announcement), &base, "");
    if let Some(parent) = crate::lineage::ForkOf::of(announcement) {
        let parent = parent.address();
        out.push_str(&format!(
            "<p class=\"forked-from\">{}Forked from <a href=\"{}\">{} / {}</a></p>",
            svg(icon::FORK, "icon-sm"),
            escape(&parent.repo_path()),
            name(names, parent.owner()),
            escape(parent.d()),
        ));
    }
    if let Some(description) = first_tag(announcement, "description")
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        out.push_str(&format!(
            "<p class=\"repo-desc\">{}</p>",
            escape(description)
        ));
    }
    let mut tabs: Vec<chrome::Tab> = [
        (Tab::Code, "Code", icon::CODE, base.clone()),
        (Tab::Issues, "Issues", icon::ISSUE, format!("{base}/issues")),
        (
            Tab::Pulls,
            "Pull requests",
            icon::PULL,
            format!("{base}/pulls"),
        ),
        (
            Tab::Commits,
            "Commits",
            icon::COMMIT,
            format!("{base}/commits"),
        ),
    ]
    .into_iter()
    .map(|(which, label, icon, href)| chrome::Tab::new(label, icon, href).current(which == tab))
    .collect();
    if apps.serves(App::Social) {
        tabs.push(chrome::Tab::new(
            "Social",
            icon::COMMENT,
            social_path(owner, d),
        ));
    }
    out.push_str(&chrome::tabs("Repository", &tabs));
    out
}

/// The repo's page on `/social`.
fn social_path(owner: &PublicKey, d: &str) -> String {
    format!(
        "{}/r/{}/{}",
        App::Social.prefix(),
        crate::npub(owner),
        crate::percent_encode(d)
    )
}

/// The branch menu: a button naming `current` that opens the branches and
/// tags, each linking to `page` (`tree` or `commits`) at that ref.
pub(crate) fn ref_menu(base: &str, page: &str, current: &str, state: &Event) -> String {
    let mut branches = Vec::new();
    let mut tags = Vec::new();
    for (name, _) in crate::refs(state) {
        if let Some(branch) = name.strip_prefix("refs/heads/") {
            branches.push(branch);
        } else if let Some(tag) = name.strip_prefix("refs/tags/") {
            tags.push(tag);
        }
    }
    let shown = if current.len() == 40 && current.bytes().all(|b| b.is_ascii_hexdigit()) {
        &current[..7]
    } else {
        current
    };
    let mut out = format!(
        "<details class=\"menu\"><summary class=\"btn mono\">{}{}{}</summary>\
         <div class=\"menu-panel\">",
        svg(icon::BRANCH, "icon-sm"),
        escape(shown),
        svg(icon::CHEVRON, "icon-sm caret"),
    );
    for (title, names) in [("Branches", branches), ("Tags", tags)] {
        if names.is_empty() {
            continue;
        }
        out.push_str(&format!("<div class=\"menu-title\">{title}</div>"));
        for name in names {
            out.push_str(&format!(
                "<a class=\"menu-item{}\" href=\"{}/{page}/{}\">{}</a>",
                if name == current { " current" } else { "" },
                escape(base),
                escape(
                    &name
                        .split('/')
                        .map(crate::percent_encode)
                        .collect::<Vec<_>>()
                        .join("/")
                ),
                escape(name),
            ));
        }
    }
    out.push_str("</div></details>");
    out
}

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent, Kind, Tag};
    use nostr::key::Keys;

    use super::*;

    #[test]
    fn the_repo_header_links_to_social_only_when_it_is_on() {
        let owner = Keys::generate();
        let announcement = EventBuilder::new(Kind::from(30_617), "")
            .tags([Tag::parse(["d", "r"]).unwrap()])
            .finalize(&owner)
            .unwrap();
        let header = |apps: &Apps| {
            repo_header(
                apps,
                &owner.public_key(),
                "r",
                &announcement,
                &HashMap::new(),
                Tab::Code,
            )
        };
        let on = header(&Apps::new([App::Repo, App::Social]));
        assert!(on.contains(&format!(
            "href=\"/social/r/{}/r\"",
            crate::npub(&owner.public_key())
        )));
        assert!(!header(&Apps::new([App::Repo])).contains("/social"));
    }

    #[test]
    fn a_fork_says_what_it_was_forked_from() {
        let (owner, parent) = (Keys::generate(), Keys::generate());
        let parent_d = "harmonics";
        let announcement = EventBuilder::new(Kind::from(30_617), "")
            .tags([
                Tag::parse(["d", "r"]).unwrap(),
                Tag::parse([
                    "u".to_owned(),
                    format!(
                        "30617:{}:{parent_d}|https://git.test/h.git",
                        parent.public_key().to_hex()
                    ),
                ])
                .unwrap(),
            ])
            .finalize(&owner)
            .unwrap();
        let names = HashMap::from([(parent.public_key(), "noaa".to_owned())]);
        let header = repo_header(
            &Apps::new([App::Repo]),
            &owner.public_key(),
            "r",
            &announcement,
            &names,
            Tab::Code,
        );
        assert!(header.contains(&format!(
            "Forked from <a href=\"/repo/{}/harmonics\">",
            crate::npub(&parent.public_key())
        )));
        assert!(header.contains(">noaa</span> / harmonics</a>"));
    }
}
