//! The pieces every page is drawn from, in rig-web's look: the page shell,
//! the repo header and its tabs, icons, avatars and dates. The look itself is
//! `relay_ui`'s stylesheet; a page only names its classes.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use nostr::event::Event;
use nostr::key::PublicKey;
use nostr::types::Timestamp;

use crate::{PREFIX, escape, first_tag, npub, repo_name, repo_path};

/// Octicons (MIT, GitHub), 16px.
pub(crate) mod icon {
    pub(crate) const REPO: &str = "M2 2.5A2.5 2.5 0 0 1 4.5 0h8.75a.75.75 0 0 1 .75.75v12.5a.75.75 0 0 1-.75.75h-2.5a.75.75 0 0 1 0-1.5h1.75v-2h-8a1 1 0 0 0-.714 1.7.75.75 0 1 1-1.072 1.05A2.495 2.495 0 0 1 2 11.5Zm10.5-1h-8a1 1 0 0 0-1 1v6.708A2.486 2.486 0 0 1 4.5 9h8ZM5 12.25a.25.25 0 0 1 .25-.25h3.5a.25.25 0 0 1 .25.25v3.25a.25.25 0 0 1-.4.2l-1.45-1.087a.249.249 0 0 0-.3 0L5.4 15.7a.25.25 0 0 1-.4-.2Z";
    pub(crate) const CODE: &str = "M4.72 3.22a.75.75 0 011.06 1.06L2.06 8l3.72 3.72a.75.75 0 11-1.06 1.06L.47 8.53a.75.75 0 010-1.06l4.25-4.25zm6.56 0a.75.75 0 10-1.06 1.06L13.94 8l-3.72 3.72a.75.75 0 101.06 1.06l4.25-4.25a.75.75 0 000-1.06L11.28 3.22z";
    pub(crate) const ISSUE: &str = "M8 9.5a1.5 1.5 0 100-3 1.5 1.5 0 000 3z M8 0a8 8 0 100 16A8 8 0 008 0zM1.5 8a6.5 6.5 0 1113 0 6.5 6.5 0 01-13 0z";
    pub(crate) const ISSUE_DONE: &str = "M11.28 6.78a.75.75 0 0 0-1.06-1.06L7.25 8.69 5.78 7.22a.75.75 0 0 0-1.06 1.06l2 2a.75.75 0 0 0 1.06 0l3.5-3.5Z M16 8A8 8 0 1 1 0 8a8 8 0 0 1 16 0Zm-1.5 0a6.5 6.5 0 1 0-13 0 6.5 6.5 0 0 0 13 0Z";
    pub(crate) const ISSUE_CLOSED: &str = "M8 0a8 8 0 1 1 0 16A8 8 0 0 1 8 0ZM1.5 8a6.5 6.5 0 1 0 13 0 6.5 6.5 0 0 0-13 0Zm9.78-2.22-5.5 5.5a.749.749 0 0 1-1.275-.326.749.749 0 0 1 .215-.734l5.5-5.5a.751.751 0 0 1 1.042.018.751.751 0 0 1 .018 1.042Z";
    pub(crate) const PULL: &str = "M1.5 3.25a2.25 2.25 0 113 2.122v5.256a2.251 2.251 0 11-1.5 0V5.372A2.25 2.25 0 011.5 3.25zm5.677-.177L9.573.677A.25.25 0 0110 .854V2.5h1A2.5 2.5 0 0113.5 5v5.628a2.251 2.251 0 11-1.5 0V5a1 1 0 00-1-1h-1v1.646a.25.25 0 01-.427.177L7.177 3.427a.25.25 0 010-.354zM3.75 2.5a.75.75 0 100 1.5.75.75 0 000-1.5zm0 9.5a.75.75 0 100 1.5.75.75 0 000-1.5zm8.25.75a.75.75 0 11-1.5 0 .75.75 0 011.5 0z";
    pub(crate) const MERGED: &str = "M5.45 5.154A4.25 4.25 0 0 0 9.25 7.5h1.378a2.251 2.251 0 1 1 0 1.5H9.25A5.734 5.734 0 0 1 5 7.123v3.505a2.25 2.25 0 1 1-1.5 0V5.372a2.25 2.25 0 1 1 1.95-.218ZM4.25 13.5a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5Zm8.5-4.5a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5ZM5 3.25a.75.75 0 1 0 0 .005V3.25Z";
    pub(crate) const PULL_CLOSED: &str = "M3.25 1A2.25 2.25 0 0 1 4 5.372v5.256a2.251 2.251 0 1 1-1.5 0V5.372A2.251 2.251 0 0 1 3.25 1Zm9.5 5.5a.75.75 0 0 1 .75.75v3.378a2.251 2.251 0 1 1-1.5 0V7.25a.75.75 0 0 1 .75-.75Zm-2.03-5.273a.75.75 0 0 1 1.06 0l.97.97.97-.97a.748.748 0 0 1 1.265.332.75.75 0 0 1-.205.729l-.97.97.97.97a.751.751 0 0 1-.018 1.042.751.751 0 0 1-1.042.018l-.97-.97-.97.97a.749.749 0 0 1-1.275-.326.749.749 0 0 1 .215-.734l.97-.97-.97-.97a.75.75 0 0 1 0-1.06ZM2.5 12.25a.75.75 0 1 0 1.5 0 .75.75 0 0 0-1.5 0Zm0-9a.75.75 0 1 0 1.5 0 .75.75 0 0 0-1.5 0Zm9.5 9a.75.75 0 1 0 1.5 0 .75.75 0 0 0-1.5 0Z";
    pub(crate) const COMMIT: &str = "M11.93 8.5a4.002 4.002 0 01-7.86 0H.75a.75.75 0 010-1.5h3.32a4.002 4.002 0 017.86 0h3.32a.75.75 0 010 1.5h-3.32zm-1.43-.75a2.5 2.5 0 10-5 0 2.5 2.5 0 005 0z";
    pub(crate) const BRANCH: &str = "M9.5 3.25a2.25 2.25 0 1 1 3 2.122V6A2.5 2.5 0 0 1 10 8.5H6a1 1 0 0 0-1 1v1.128a2.251 2.251 0 1 1-1.5 0V5.372a2.25 2.25 0 1 1 1.5 0v1.836A2.493 2.493 0 0 1 6 7h4a1 1 0 0 0 1-1v-.628A2.25 2.25 0 0 1 9.5 3.25Zm-6 0a.75.75 0 1 0 1.5 0 .75.75 0 0 0-1.5 0Zm8.25-.75a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5ZM4.25 12a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5Z";
    pub(crate) const TAG: &str = "M1 7.775V2.75C1 1.784 1.784 1 2.75 1h5.025c.464 0 .91.184 1.238.513l6.25 6.25a1.75 1.75 0 0 1 0 2.474l-5.026 5.026a1.75 1.75 0 0 1-2.474 0l-6.25-6.25A1.752 1.752 0 0 1 1 7.775Zm1.5 0c0 .066.026.13.073.177l6.25 6.25a.25.25 0 0 0 .354 0l5.025-5.025a.25.25 0 0 0 0-.354l-6.25-6.25a.25.25 0 0 0-.177-.073H2.75a.25.25 0 0 0-.25.25ZM6 5a1 1 0 1 1 0 2 1 1 0 0 1 0-2Z";
    pub(crate) const LINK: &str = "m7.775 3.275 1.25-1.25a3.5 3.5 0 1 1 4.95 4.95l-2.5 2.5a3.5 3.5 0 0 1-4.95 0 .751.751 0 0 1 .018-1.042.751.751 0 0 1 1.042-.018 1.998 1.998 0 0 0 2.83 0l2.5-2.5a2.002 2.002 0 0 0-2.83-2.83l-1.25 1.25a.751.751 0 0 1-1.042-.018.751.751 0 0 1-.018-1.042Zm-4.69 9.64a1.998 1.998 0 0 0 2.83 0l1.25-1.25a.751.751 0 0 1 1.042.018.751.751 0 0 1 .018 1.042l-1.25 1.25a3.5 3.5 0 1 1-4.95-4.95l2.5-2.5a3.5 3.5 0 0 1 4.95 0 .751.751 0 0 1-.018 1.042.751.751 0 0 1-1.042.018 1.998 1.998 0 0 0-2.83 0l-2.5 2.5a1.998 1.998 0 0 0 0 2.83Z";
    pub(crate) const CHEVRON: &str = "M12.78 5.22a.749.749 0 0 1 0 1.06l-4.25 4.25a.749.749 0 0 1-1.06 0L3.22 6.28a.749.749 0 1 1 1.06-1.06L8 8.94l3.72-3.72a.749.749 0 0 1 1.06 0Z";
}

/// An icon, its path drawn in the text colour.
pub(crate) fn svg(path: &str, class: &str) -> String {
    format!(
        "<svg class=\"icon {class}\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path d=\"{path}\"/></svg>"
    )
}

/// The whole page: the stylesheet, a bar back to the list, and `body`.
pub(crate) fn page(title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{}</title><link rel=\"stylesheet\" href=\"{PREFIX}/assets/{}\"></head>\
         <body><header class=\"topbar\"><div class=\"container\">{}\
         <a href=\"{PREFIX}\">Repositories</a></div></header>\
         <main class=\"container stack\">{body}</main></body></html>",
        escape(title),
        relay_ui::stylesheet_name(),
        svg(icon::REPO, ""),
    )
}

/// A person's name: the profile's where there is one, else the start of the
/// npub. The full npub is the title either way.
pub(crate) fn name(names: &HashMap<PublicKey, String>, key: &PublicKey) -> String {
    let npub = npub(key);
    let shown = match names.get(key) {
        Some(name) => name.clone(),
        None => format!("{}…", &npub[..12]),
    };
    format!(
        "<span class=\"strong\" title=\"{}\">{}</span>",
        escape(&npub),
        escape(&shown)
    )
}

/// The plain text of a person's name, for initials.
pub(crate) fn plain_name(names: &HashMap<PublicKey, String>, key: &PublicKey) -> String {
    names
        .get(key)
        .cloned()
        .unwrap_or_else(|| npub(key)[5..7].to_owned())
}

/// A round avatar holding the first two letters of `name`.
pub(crate) fn avatar(name: &str, small: bool) -> String {
    let initials: String = name
        .chars()
        .filter(|c| c.is_alphanumeric())
        .take(2)
        .collect::<String>()
        .to_uppercase();
    format!(
        "<span class=\"avatar{}\" aria-hidden=\"true\">{}</span>",
        if small { " avatar-sm" } else { "" },
        escape(&initials)
    )
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `(year, month 1-12, day)` of a Unix time, in UTC.
fn civil(seconds: i64) -> (i64, usize, i64) {
    let days = seconds.div_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, usize::try_from(month).unwrap_or(1), day)
}

/// `Oct 21, 2026`.
pub(crate) fn date(seconds: i64) -> String {
    let (year, month, day) = civil(seconds);
    format!("{} {day}, {year}", MONTHS[month.clamp(1, 12) - 1])
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(0))
}

/// How long ago `then` was, as GitHub says it, with the date as its title.
pub(crate) fn ago(then: Timestamp) -> String {
    let then = i64::try_from(then.as_secs()).unwrap_or(0);
    format!(
        "<time title=\"{}\">{}</time>",
        date(then),
        relative(then, now())
    )
}

/// `then` relative to `now`. A pure function.
pub(crate) fn relative(then: i64, now: i64) -> String {
    let seconds = (now - then).max(0);
    let plural = |n: i64, unit: &str| format!("{n} {unit}{} ago", if n == 1 { "" } else { "s" });
    match seconds {
        0..60 => "just now".to_owned(),
        60..3_600 => plural(seconds / 60, "minute"),
        3_600..86_400 => plural(seconds / 3_600, "hour"),
        86_400..172_800 => "yesterday".to_owned(),
        172_800..2_592_000 => plural(seconds / 86_400, "day"),
        _ => format!("on {}", date(then)),
    }
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
/// description and the tabs, with `tab` current.
pub(crate) fn repo_header(
    owner: &PublicKey,
    d: &str,
    announcement: &Event,
    names: &HashMap<PublicKey, String>,
    tab: Tab,
) -> String {
    let base = repo_path(owner, d);
    let mut out = format!(
        "<div class=\"repo-head\">{}{}<span class=\"sep\">/</span>\
         <h1><a class=\"name\" href=\"{}\">{}</a></h1><span class=\"pill\">Public</span></div>",
        svg(icon::REPO, ""),
        name(names, owner),
        escape(&base),
        escape(repo_name(announcement)),
    );
    if let Some(description) = first_tag(announcement, "description")
        .map(str::trim)
        .filter(|text| !text.is_empty())
    {
        out.push_str(&format!(
            "<p class=\"repo-desc\">{}</p>",
            escape(description)
        ));
    }
    out.push_str("<nav class=\"tabs\" aria-label=\"Repository\">");
    for (which, label, icon, href) in [
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
    ] {
        let current = which == tab;
        out.push_str(&format!(
            "<a class=\"tab{}\" href=\"{}\"{}>{}{label}</a>",
            if current { " active" } else { "" },
            escape(&href),
            if current {
                " aria-current=\"page\""
            } else {
                ""
            },
            svg(icon, ""),
        ));
    }
    out.push_str("</nav>");
    out
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
    use super::*;

    #[test]
    fn dates_read_as_github_says_them() {
        assert_eq!(date(0), "Jan 1, 1970");
        assert_eq!(date(1_792_540_800), "Oct 21, 2026");
        let now = 1_792_540_800;
        assert_eq!(relative(now - 5, now), "just now");
        assert_eq!(relative(now - 60, now), "1 minute ago");
        assert_eq!(relative(now - 7_200, now), "2 hours ago");
        assert_eq!(relative(now - 90_000, now), "yesterday");
        assert_eq!(relative(now - 3 * 86_400, now), "3 days ago");
        assert_eq!(relative(0, now), "on Jan 1, 1970");
        assert_eq!(relative(now + 100, now), "just now");
    }

    #[test]
    fn initials_are_the_first_two_letters() {
        assert!(avatar("ada lovelace", false).contains(">AD<"));
        assert!(avatar("<b>", true).contains(">B<"));
    }
}
