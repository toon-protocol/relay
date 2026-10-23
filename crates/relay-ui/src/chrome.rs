//! The chrome every app UI page is drawn from, so `/repo` and `/social` read
//! as one site: the page shell with its top bar and app switcher, the repo
//! header's title and tabs, icons, people's names and avatars, and dates.
//!
//! Every function here writes escaped HTML that names classes of the shared
//! stylesheet (`style.css`) and nothing else: the pages' policy refuses
//! `style` attributes and inline script. An app adds its own markup between
//! these pieces; what it shares with another app lives here, once.

use std::collections::{HashMap, HashSet};
use std::time::{SystemTime, UNIX_EPOCH};

use nostr::event::{Event, Kind};
use nostr::filter::Filter;
use nostr::key::PublicKey;
use nostr::nips::nip19::ToBech32;
use nostr::types::Timestamp;

use crate::ReadOnlyStore;

/// An app UI the relay can serve, by the name `TOON_UI_APPS` gives it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum App {
    /// NIP-34 git repositories, at `/repo`.
    Repo,
    /// The social NIPs, repos first, at `/social`.
    Social,
}

impl App {
    /// Every app, in the order the switcher shows them.
    pub const ALL: [App; 2] = [App::Repo, App::Social];

    /// The name `TOON_UI_APPS` gives the app.
    pub fn name(self) -> &'static str {
        match self {
            Self::Repo => "repo",
            Self::Social => "social",
        }
    }

    /// The app called `name` in `TOON_UI_APPS`, if there is one.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|app| app.name() == name)
    }

    /// The mount prefix: a slash and a name.
    pub fn prefix(self) -> &'static str {
        match self {
            Self::Repo => "/repo",
            Self::Social => "/social",
        }
    }

    /// What the switcher calls it.
    pub fn label(self) -> &'static str {
        match self {
            Self::Repo => "Repos",
            Self::Social => "Social",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Repo => icon::REPO,
            Self::Social => icon::COMMENT,
        }
    }
}

/// The apps the relay serves: what the switcher offers and which cross-links
/// a page may draw. A link to an app that is off would answer `426`, so a
/// page asks [`Apps::serves`] before drawing one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Apps {
    on: Vec<App>,
}

impl Apps {
    /// The apps in `on`, once each, in the switcher's order.
    pub fn new(on: impl IntoIterator<Item = App>) -> Self {
        let on: Vec<App> = on.into_iter().collect();
        Self {
            on: App::ALL
                .into_iter()
                .filter(|app| on.contains(app))
                .collect(),
        }
    }

    /// Whether `app` is served.
    pub fn serves(&self, app: App) -> bool {
        self.on.contains(&app)
    }
}

/// Octicons (MIT, GitHub), 16px, drawn in the text colour by [`svg`].
pub mod icon {
    pub const REPO: &str = "M2 2.5A2.5 2.5 0 0 1 4.5 0h8.75a.75.75 0 0 1 .75.75v12.5a.75.75 0 0 1-.75.75h-2.5a.75.75 0 0 1 0-1.5h1.75v-2h-8a1 1 0 0 0-.714 1.7.75.75 0 1 1-1.072 1.05A2.495 2.495 0 0 1 2 11.5Zm10.5-1h-8a1 1 0 0 0-1 1v6.708A2.486 2.486 0 0 1 4.5 9h8ZM5 12.25a.25.25 0 0 1 .25-.25h3.5a.25.25 0 0 1 .25.25v3.25a.25.25 0 0 1-.4.2l-1.45-1.087a.249.249 0 0 0-.3 0L5.4 15.7a.25.25 0 0 1-.4-.2Z";
    pub const CODE: &str = "M4.72 3.22a.75.75 0 011.06 1.06L2.06 8l3.72 3.72a.75.75 0 11-1.06 1.06L.47 8.53a.75.75 0 010-1.06l4.25-4.25zm6.56 0a.75.75 0 10-1.06 1.06L13.94 8l-3.72 3.72a.75.75 0 101.06 1.06l4.25-4.25a.75.75 0 000-1.06L11.28 3.22z";
    pub const ISSUE: &str = "M8 9.5a1.5 1.5 0 100-3 1.5 1.5 0 000 3z M8 0a8 8 0 100 16A8 8 0 008 0zM1.5 8a6.5 6.5 0 1113 0 6.5 6.5 0 01-13 0z";
    pub const ISSUE_DONE: &str = "M11.28 6.78a.75.75 0 0 0-1.06-1.06L7.25 8.69 5.78 7.22a.75.75 0 0 0-1.06 1.06l2 2a.75.75 0 0 0 1.06 0l3.5-3.5Z M16 8A8 8 0 1 1 0 8a8 8 0 0 1 16 0Zm-1.5 0a6.5 6.5 0 1 0-13 0 6.5 6.5 0 0 0 13 0Z";
    pub const ISSUE_CLOSED: &str = "M8 0a8 8 0 1 1 0 16A8 8 0 0 1 8 0ZM1.5 8a6.5 6.5 0 1 0 13 0 6.5 6.5 0 0 0-13 0Zm9.78-2.22-5.5 5.5a.749.749 0 0 1-1.275-.326.749.749 0 0 1 .215-.734l5.5-5.5a.751.751 0 0 1 1.042.018.751.751 0 0 1 .018 1.042Z";
    pub const PULL: &str = "M1.5 3.25a2.25 2.25 0 113 2.122v5.256a2.251 2.251 0 11-1.5 0V5.372A2.25 2.25 0 011.5 3.25zm5.677-.177L9.573.677A.25.25 0 0110 .854V2.5h1A2.5 2.5 0 0113.5 5v5.628a2.251 2.251 0 11-1.5 0V5a1 1 0 00-1-1h-1v1.646a.25.25 0 01-.427.177L7.177 3.427a.25.25 0 010-.354zM3.75 2.5a.75.75 0 100 1.5.75.75 0 000-1.5zm0 9.5a.75.75 0 100 1.5.75.75 0 000-1.5zm8.25.75a.75.75 0 11-1.5 0 .75.75 0 011.5 0z";
    pub const MERGED: &str = "M5.45 5.154A4.25 4.25 0 0 0 9.25 7.5h1.378a2.251 2.251 0 1 1 0 1.5H9.25A5.734 5.734 0 0 1 5 7.123v3.505a2.25 2.25 0 1 1-1.5 0V5.372a2.25 2.25 0 1 1 1.95-.218ZM4.25 13.5a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5Zm8.5-4.5a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5ZM5 3.25a.75.75 0 1 0 0 .005V3.25Z";
    pub const PULL_CLOSED: &str = "M3.25 1A2.25 2.25 0 0 1 4 5.372v5.256a2.251 2.251 0 1 1-1.5 0V5.372A2.251 2.251 0 0 1 3.25 1Zm9.5 5.5a.75.75 0 0 1 .75.75v3.378a2.251 2.251 0 1 1-1.5 0V7.25a.75.75 0 0 1 .75-.75Zm-2.03-5.273a.75.75 0 0 1 1.06 0l.97.97.97-.97a.748.748 0 0 1 1.265.332.75.75 0 0 1-.205.729l-.97.97.97.97a.751.751 0 0 1-.018 1.042.751.751 0 0 1-1.042.018l-.97-.97-.97.97a.749.749 0 0 1-1.275-.326.749.749 0 0 1 .215-.734l.97-.97-.97-.97a.75.75 0 0 1 0-1.06ZM2.5 12.25a.75.75 0 1 0 1.5 0 .75.75 0 0 0-1.5 0Zm0-9a.75.75 0 1 0 1.5 0 .75.75 0 0 0-1.5 0Zm9.5 9a.75.75 0 1 0 1.5 0 .75.75 0 0 0-1.5 0Z";
    pub const COMMIT: &str = "M11.93 8.5a4.002 4.002 0 01-7.86 0H.75a.75.75 0 010-1.5h3.32a4.002 4.002 0 017.86 0h3.32a.75.75 0 010 1.5h-3.32zm-1.43-.75a2.5 2.5 0 10-5 0 2.5 2.5 0 005 0z";
    pub const BRANCH: &str = "M9.5 3.25a2.25 2.25 0 1 1 3 2.122V6A2.5 2.5 0 0 1 10 8.5H6a1 1 0 0 0-1 1v1.128a2.251 2.251 0 1 1-1.5 0V5.372a2.25 2.25 0 1 1 1.5 0v1.836A2.493 2.493 0 0 1 6 7h4a1 1 0 0 0 1-1v-.628A2.25 2.25 0 0 1 9.5 3.25Zm-6 0a.75.75 0 1 0 1.5 0 .75.75 0 0 0-1.5 0Zm8.25-.75a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5ZM4.25 12a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5Z";
    pub const TAG: &str = "M1 7.775V2.75C1 1.784 1.784 1 2.75 1h5.025c.464 0 .91.184 1.238.513l6.25 6.25a1.75 1.75 0 0 1 0 2.474l-5.026 5.026a1.75 1.75 0 0 1-2.474 0l-6.25-6.25A1.752 1.752 0 0 1 1 7.775Zm1.5 0c0 .066.026.13.073.177l6.25 6.25a.25.25 0 0 0 .354 0l5.025-5.025a.25.25 0 0 0 0-.354l-6.25-6.25a.25.25 0 0 0-.177-.073H2.75a.25.25 0 0 0-.25.25ZM6 5a1 1 0 1 1 0 2 1 1 0 0 1 0-2Z";
    pub const LINK: &str = "m7.775 3.275 1.25-1.25a3.5 3.5 0 1 1 4.95 4.95l-2.5 2.5a3.5 3.5 0 0 1-4.95 0 .751.751 0 0 1 .018-1.042.751.751 0 0 1 1.042-.018 1.998 1.998 0 0 0 2.83 0l2.5-2.5a2.002 2.002 0 0 0-2.83-2.83l-1.25 1.25a.751.751 0 0 1-1.042-.018.751.751 0 0 1-.018-1.042Zm-4.69 9.64a1.998 1.998 0 0 0 2.83 0l1.25-1.25a.751.751 0 0 1 1.042.018.751.751 0 0 1 .018 1.042l-1.25 1.25a3.5 3.5 0 1 1-4.95-4.95l2.5-2.5a3.5 3.5 0 0 1 4.95 0 .751.751 0 0 1-.018 1.042.751.751 0 0 1-1.042.018 1.998 1.998 0 0 0-2.83 0l-2.5 2.5a1.998 1.998 0 0 0 0 2.83Z";
    pub const CHEVRON: &str = "M12.78 5.22a.749.749 0 0 1 0 1.06l-4.25 4.25a.749.749 0 0 1-1.06 0L3.22 6.28a.749.749 0 1 1 1.06-1.06L8 8.94l3.72-3.72a.749.749 0 0 1 1.06 0Z";
    pub const STAR: &str = "M8 .25a.75.75 0 0 1 .673.418l1.882 3.815 4.21.612a.75.75 0 0 1 .416 1.279l-3.046 2.97.719 4.192a.751.751 0 0 1-1.088.791L8 12.347l-3.766 1.98a.75.75 0 0 1-1.088-.79l.72-4.194L.818 6.374a.75.75 0 0 1 .416-1.28l4.21-.611L7.327.668A.75.75 0 0 1 8 .25Zm0 2.445L6.615 5.5a.75.75 0 0 1-.564.41l-3.097.45 2.24 2.184a.75.75 0 0 1 .216.664l-.528 3.084 2.769-1.456a.75.75 0 0 1 .698 0l2.77 1.456-.53-3.084a.75.75 0 0 1 .216-.664l2.24-2.183-3.096-.45a.75.75 0 0 1-.564-.41L8 2.694Z";
    pub const EYE: &str = "M8 2c1.981 0 3.671.992 4.933 2.078 1.27 1.091 2.187 2.345 2.637 3.023a1.62 1.62 0 0 1 0 1.798c-.45.678-1.367 1.932-2.637 3.023C11.67 13.008 9.981 14 8 14c-1.981 0-3.671-.992-4.933-2.078C1.797 10.83.88 9.576.43 8.898a1.62 1.62 0 0 1 0-1.798c.45-.677 1.367-1.931 2.637-3.022C4.33 2.992 6.019 2 8 2ZM1.679 7.932a.12.12 0 0 0 0 .136c.411.622 1.241 1.75 2.366 2.717C5.176 11.758 6.527 12.5 8 12.5c1.473 0 2.825-.742 3.955-1.715 1.124-.967 1.954-2.096 2.366-2.717a.12.12 0 0 0 0-.136c-.412-.621-1.242-1.75-2.366-2.717C10.824 4.242 9.473 3.5 8 3.5c-1.473 0-2.825.742-3.955 1.715-1.124.967-1.954 2.096-2.366 2.717ZM8 10a2 2 0 1 1-.001-3.999A2 2 0 0 1 8 10Z";
    pub const PLAY: &str = "M8 0a8 8 0 1 1 0 16A8 8 0 0 1 8 0ZM1.5 8a6.5 6.5 0 1 0 13 0 6.5 6.5 0 0 0-13 0Zm4.879-2.773 4.264 2.559a.25.25 0 0 1 0 .428l-4.264 2.559A.25.25 0 0 1 6 10.559V5.442a.25.25 0 0 1 .379-.215Z";
    pub const REPOST: &str = "M5.22 14.78a.75.75 0 0 0 1.06-1.06L4.56 12h8.69a.75.75 0 0 0 0-1.5H4.56l1.72-1.72a.75.75 0 0 0-1.06-1.06l-3 3a.75.75 0 0 0 0 1.06l3 3Zm5.56-6.5a.75.75 0 1 1-1.06-1.06l1.72-1.72H2.75a.75.75 0 0 1 0-1.5h8.69L9.72 2.28a.75.75 0 0 1 1.06-1.06l3 3a.75.75 0 0 1 0 1.06l-3 3Z";
    pub const COMMENT: &str = "M1 2.75C1 1.784 1.784 1 2.75 1h10.5c.966 0 1.75.784 1.75 1.75v7.5A1.75 1.75 0 0 1 13.25 12H9.06l-2.573 2.573A1.458 1.458 0 0 1 4 13.543V12H2.75A1.75 1.75 0 0 1 1 10.25Zm1.75-.25a.25.25 0 0 0-.25.25v7.5c0 .138.112.25.25.25h2a.75.75 0 0 1 .75.75v2.19l2.72-2.72a.749.749 0 0 1 .53-.22h4.5a.25.25 0 0 0 .25-.25v-7.5a.25.25 0 0 0-.25-.25Z";
    pub const IMAGE: &str = "M16 13.25A1.75 1.75 0 0 1 14.25 15H1.75A1.75 1.75 0 0 1 0 13.25V2.75C0 1.784.784 1 1.75 1h12.5c.966 0 1.75.784 1.75 1.75ZM1.75 2.5a.25.25 0 0 0-.25.25v10.5c0 .138.112.25.25.25h.94l.03-.03 6.077-6.078a1.75 1.75 0 0 1 2.412-.06L14.5 10.31V2.75a.25.25 0 0 0-.25-.25Zm12.5 11a.25.25 0 0 0 .25-.25v-.917l-4.298-3.889a.25.25 0 0 0-.344.009L4.81 13.5ZM7 6a2 2 0 1 1-3.999.001A2 2 0 0 1 7 6ZM5.5 6a.5.5 0 1 0-1 0 .5.5 0 0 0 1 0Z";
    pub const PERSON: &str = "M10.561 8.073a6.005 6.005 0 0 1 3.432 5.142.75.75 0 1 1-1.498.07 4.5 4.5 0 0 0-8.99 0 .75.75 0 0 1-1.498-.07 6.004 6.004 0 0 1 3.431-5.142 3.999 3.999 0 1 1 5.123 0ZM10.5 5a2.5 2.5 0 1 0-5 0 2.5 2.5 0 0 0 5 0Z";
    pub const HOME: &str = "M6.906.664a1.749 1.749 0 0 1 2.187 0l5.25 4.2c.415.332.657.835.657 1.367v7.019A1.75 1.75 0 0 1 13.25 15h-3.5a.75.75 0 0 1-.75-.75V9H7v5.25a.75.75 0 0 1-.75.75h-3.5A1.75 1.75 0 0 1 1 13.25V6.23c0-.531.242-1.034.657-1.366l5.25-4.2Zm1.25 1.171a.25.25 0 0 0-.312 0l-5.25 4.2a.25.25 0 0 0-.094.196v7.019c0 .138.112.25.25.25H5.5V8.25a.75.75 0 0 1 .75-.75h3.5a.75.75 0 0 1 .75.75v5.25h2.75a.25.25 0 0 0 .25-.25V6.23a.25.25 0 0 0-.094-.195Z";
    /// An open book: articles.
    pub const BOOK: &str = "M0 1.75A.75.75 0 0 1 .75 1h4.253c1.227 0 2.317.59 3 1.501A3.743 3.743 0 0 1 11.006 1h4.245a.75.75 0 0 1 .75.75v10.5a.75.75 0 0 1-.75.75h-4.507a2.25 2.25 0 0 0-1.591.659l-.622.621a.75.75 0 0 1-1.06 0l-.622-.621A2.25 2.25 0 0 0 5.258 13H.75a.75.75 0 0 1-.75-.75Zm7.251 10.324.004-5.073-.002-2.253A2.25 2.25 0 0 0 5.003 2.5H1.5v9h3.757a3.75 3.75 0 0 1 1.994.574ZM8.755 4.75l-.004 7.322a3.752 3.752 0 0 1 1.992-.572H14.5v-9h-3.495a2.25 2.25 0 0 0-2.25 2.25Z";
    /// A flag on a pole: a ship post.
    pub const SHIP: &str = "M3 1.75a.75.75 0 0 1 1.5 0V2h8.75a.75.75 0 0 1 .6 1.2L11.94 5.75l1.91 2.55a.75.75 0 0 1-.6 1.2H4.5v5.75a.75.75 0 0 1-1.5 0Zm1.5 6.25h7.25L10.4 6.2a.75.75 0 0 1 0-.9l1.35-1.8H4.5Z";
    /// A repo forked: forks and remixes.
    pub const FORK: &str = "M5 5.372v.878c0 .414.336.75.75.75h4.5a.75.75 0 0 0 .75-.75v-.878a2.25 2.25 0 1 1 1.5 0v.878a2.25 2.25 0 0 1-2.25 2.25h-1.5v2.128a2.251 2.251 0 1 1-1.5 0V8.5h-1.5A2.25 2.25 0 0 1 3.5 6.25v-.878a2.25 2.25 0 1 1 1.5 0ZM5 3.25a.75.75 0 1 0-1.5 0 .75.75 0 0 0 1.5 0Zm6.75.75a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5Zm-3 8.75a.75.75 0 1 0-1.5 0 .75.75 0 0 0 1.5 0Z";
    /// A calendar page: an event.
    pub const CALENDAR: &str = "M4.75 0a.75.75 0 0 1 .75.75V2h5V.75a.75.75 0 0 1 1.5 0V2h1.25c.966 0 1.75.784 1.75 1.75v10.5A1.75 1.75 0 0 1 13.25 16H2.75A1.75 1.75 0 0 1 1 14.25V3.75C1 2.784 1.784 2 2.75 2H4V.75A.75.75 0 0 1 4.75 0ZM2.5 7.5v6.75c0 .138.112.25.25.25h10.5a.25.25 0 0 0 .25-.25V7.5Zm10.75-4H2.75a.25.25 0 0 0-.25.25V6h11V3.75a.25.25 0 0 0-.25-.25Z";
    /// A map pin: where an event happens.
    pub const LOCATION: &str = "m12.596 11.596-3.535 3.536a1.5 1.5 0 0 1-2.122 0l-3.535-3.536a6.5 6.5 0 1 1 9.192-9.193 6.5 6.5 0 0 1 0 9.193Zm-1.06-8.132v-.001a5 5 0 1 0-7.072 7.072L8 14.07l3.536-3.534a5 5 0 0 0 0-7.072ZM8 9a2 2 0 1 1-.001-3.999A2 2 0 0 1 8 9Z";
}

/// An icon, its path drawn in the text colour.
pub fn svg(path: &str, class: &str) -> String {
    format!(
        "<svg class=\"icon {class}\" viewBox=\"0 0 16 16\" aria-hidden=\"true\"><path d=\"{path}\"/></svg>"
    )
}

/// `text` with the five HTML metacharacters escaped, for text and attributes.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            other => out.push(other),
        }
    }
    out
}

/// `text` as one URL path segment: everything but unreserved characters is
/// percent-encoded.
pub fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// A key's `npub`.
pub fn npub(key: &PublicKey) -> String {
    let Ok(npub) = key.to_bech32();
    npub
}

/// The whole page: the stylesheet, the top bar with the app switcher (when
/// more than one app is on) and `header` after it, then `body`. `app` is the
/// app drawing the page; its stylesheet is served under its own prefix.
pub fn page(apps: &Apps, app: App, title: &str, header: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\">\
         <meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
         <title>{}</title><link rel=\"stylesheet\" href=\"{}/assets/{}\"></head>\
         <body><header class=\"topbar\"><div class=\"container\">{}{header}</div></header>\
         <main class=\"container stack\">{body}</main></body></html>",
        escape(title),
        app.prefix(),
        crate::stylesheet_name(),
        switcher(apps, app),
    )
}

/// The app switcher: one link per app that is on, `current` marked.
fn switcher(apps: &Apps, current: App) -> String {
    let mut out = String::from("<nav class=\"app-switch\" aria-label=\"Apps\">");
    for app in App::ALL
        .into_iter()
        .filter(|app| *app == current || apps.serves(*app))
    {
        let here = app == current;
        out.push_str(&format!(
            "<a class=\"app-link{}\" href=\"{}\"{}>{}{}</a>",
            if here { " current" } else { "" },
            app.prefix(),
            if here { " aria-current=\"page\"" } else { "" },
            svg(app.icon(), ""),
            app.label(),
        ));
    }
    out.push_str("</nav>");
    out
}

/// One tab of a [`tabs`] bar.
#[derive(Debug, Clone)]
pub struct Tab {
    label: String,
    icon: &'static str,
    href: String,
    current: bool,
    count: Option<usize>,
}

impl Tab {
    /// A tab called `label` linking to `href`.
    pub fn new(label: impl Into<String>, icon: &'static str, href: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            icon,
            href: href.into(),
            current: false,
            count: None,
        }
    }

    /// The tab marked as the page it is on, or not.
    pub fn current(mut self, current: bool) -> Self {
        self.current = current;
        self
    }

    /// A count shown after the label.
    pub fn count(mut self, count: usize) -> Self {
        self.count = Some(count);
        self
    }
}

/// A bar of tabs, as under a repo header.
pub fn tabs(label: &str, tabs: &[Tab]) -> String {
    let mut out = format!("<nav class=\"tabs\" aria-label=\"{}\">", escape(label));
    for tab in tabs {
        out.push_str(&format!(
            "<a class=\"tab{}\" href=\"{}\"{}>{}{}{}</a>",
            if tab.current { " active" } else { "" },
            escape(&tab.href),
            if tab.current {
                " aria-current=\"page\""
            } else {
                ""
            },
            svg(tab.icon, ""),
            escape(&tab.label),
            tab.count
                .map(|count| format!("<span class=\"count\">{count}</span>"))
                .unwrap_or_default(),
        ));
    }
    out.push_str("</nav>");
    out
}

/// A repo's title line: its glyph, `owner / name` (the name linking to
/// `href`), the visibility pill, then `actions` (buttons, or nothing).
pub fn repo_title(owner: &str, name: &str, href: &str, actions: &str) -> String {
    format!(
        "<div class=\"repo-head\">{}{owner}<span class=\"sep\">/</span>\
         <h1><a class=\"name\" href=\"{}\">{}</a></h1><span class=\"pill\">Public</span>{actions}</div>",
        svg(icon::REPO, ""),
        escape(href),
        escape(name),
    )
}

/// A person's name: the profile's where there is one, else the start of the
/// npub. The full npub is the title either way.
pub fn name(names: &HashMap<PublicKey, String>, key: &PublicKey) -> String {
    let npub = npub(key);
    let shown = shown_name(names, key);
    format!(
        "<span class=\"strong\" title=\"{}\">{}</span>",
        escape(&npub),
        escape(&shown)
    )
}

/// A key's builder profile on `/social`, `/social/p/<npub>`.
pub fn profile_path(key: &PublicKey) -> String {
    format!("{}/p/{}", App::Social.prefix(), npub(key))
}

/// A person's [`name`], linking to their `/social` profile when `apps`
/// serves `/social`; else the name alone.
pub fn person(apps: &Apps, names: &HashMap<PublicKey, String>, key: &PublicKey) -> String {
    if apps.serves(App::Social) {
        format!(
            "<a class=\"person-link\" href=\"{}\">{}</a>",
            profile_path(key),
            name(names, key)
        )
    } else {
        name(names, key)
    }
}

/// A person's [`avatar`], linking to their `/social` profile when `apps`
/// serves `/social`. The link is hidden from assistive technology: the
/// name beside it carries the same link.
pub fn person_avatar(
    apps: &Apps,
    names: &HashMap<PublicKey, String>,
    key: &PublicKey,
    small: bool,
) -> String {
    let drawn = avatar(&plain_name(names, key), small);
    if apps.serves(App::Social) {
        format!(
            "<a class=\"avatar-link\" href=\"{}\" tabindex=\"-1\" aria-hidden=\"true\">{drawn}</a>",
            profile_path(key)
        )
    } else {
        drawn
    }
}

/// The plain text [`name`] shows: the profile's name, else the start of
/// the npub.
pub fn shown_name(names: &HashMap<PublicKey, String>, key: &PublicKey) -> String {
    match names.get(key) {
        Some(name) => name.clone(),
        None => format!("{}…", &npub(key)[..12]),
    }
}

/// The plain text of a person's name, for initials.
pub fn plain_name(names: &HashMap<PublicKey, String>, key: &PublicKey) -> String {
    names
        .get(key)
        .cloned()
        .unwrap_or_else(|| npub(key)[5..7].to_owned())
}

/// A round avatar holding the first two letters of `name`. Avatars are never
/// a profile's picture: a page loads no image a person chose for themselves.
pub fn avatar(name: &str, small: bool) -> String {
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

/// What kind 0 profiles say about some keys: display names, NIP-05
/// identifiers, and which keys say they are bots (NIP-24 `"bot": true`). A
/// NIP-05 identifier is a claim; the relay never fetches the `.well-known`
/// document that would verify it, so a page shows it as one.
#[derive(Debug, Clone, Default)]
pub struct Profiles {
    names: HashMap<PublicKey, String>,
    nip05: HashMap<PublicKey, String>,
    bots: HashSet<PublicKey>,
}

impl Profiles {
    /// The newest profile of each of `keys` in the store.
    pub async fn load(store: &ReadOnlyStore, keys: &[PublicKey]) -> Self {
        if keys.is_empty() {
            return Self::default();
        }
        let events = store
            .query(Filter::new().kind(Kind::Metadata).authors(keys.to_vec()))
            .await;
        Self::from_events(keys, &events)
    }

    /// The newest of `events` for each of `keys`, read. A pure function.
    pub fn from_events(keys: &[PublicKey], events: &[Event]) -> Self {
        let mut profiles = Self::default();
        for key in keys {
            let newest = events
                .iter()
                .filter(|event| event.pubkey == *key && event.kind == Kind::Metadata)
                .max_by(|a, b| (a.created_at, b.id).cmp(&(b.created_at, a.id)));
            let Some(profile) = newest else { continue };
            let Ok(serde_json::Value::Object(fields)) = serde_json::from_str(&profile.content)
            else {
                continue;
            };
            let text = |field: &str| fields.get(field).and_then(|value| value.as_str());
            if let Some(name) = ["display_name", "name"]
                .into_iter()
                .filter_map(text)
                .find(|name| !name.trim().is_empty())
            {
                profiles.names.insert(*key, name.trim().to_owned());
            }
            if let Some(nip05) = text("nip05").and_then(nip05_identifier) {
                profiles.nip05.insert(*key, nip05);
            }
            if fields.get("bot") == Some(&serde_json::Value::Bool(true)) {
                profiles.bots.insert(*key);
            }
        }
        profiles
    }

    /// Display names, by key.
    pub fn names(&self) -> &HashMap<PublicKey, String> {
        &self.names
    }

    /// The display names alone.
    pub fn into_names(self) -> HashMap<PublicKey, String> {
        self.names
    }

    /// The NIP-05 identifier `key`'s profile claims, unverified.
    pub fn nip05(&self, key: &PublicKey) -> Option<&str> {
        self.nip05.get(key).map(String::as_str)
    }

    /// The keys whose profile says `"bot": true` (NIP-24): content that is
    /// "largely the result of automation". Only the boolean counts.
    pub fn bots(&self) -> &HashSet<PublicKey> {
        &self.bots
    }
}

/// The chip that marks a key's posts as a bot's, when `bots` holds it.
pub fn bot_chip(bots: &HashSet<PublicKey>, key: &PublicKey) -> &'static str {
    if bots.contains(key) {
        " <span class=\"chip chip-bot\" title=\"Its profile says it is a bot (NIP-24)\">bot</span>"
    } else {
        ""
    }
}

/// Display names from kind 0 profiles, by pubkey.
pub async fn names(store: &ReadOnlyStore, keys: &[PublicKey]) -> HashMap<PublicKey, String> {
    Profiles::load(store, keys).await.into_names()
}

/// A NIP-05 identifier, `local@domain` in lower case, if `claim` is one: a
/// local part of `a-z0-9-_.` and a dotted domain of `a-z0-9-.`. `_@domain`
/// is the domain itself. Anything else is none, so a claim can be written
/// into a `nostr://` URL without breaking it. A pure function.
pub fn nip05_identifier(claim: &str) -> Option<String> {
    let claim = claim.trim().to_lowercase();
    let (local, domain) = claim.split_once('@')?;
    let local_ok = !local.is_empty()
        && local
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b));
    let domain_ok = domain.contains('.')
        && !domain.starts_with(['.', '-'])
        && !domain.ends_with(['.', '-'])
        && domain
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-.".contains(&b));
    (local_ok && domain_ok).then(|| {
        if local == "_" {
            domain.to_owned()
        } else {
            claim.clone()
        }
    })
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
pub fn date(seconds: i64) -> String {
    let (year, month, day) = civil(seconds);
    format!("{} {day}, {year}", MONTHS[month.clamp(1, 12) - 1])
}

/// The current Unix time, in seconds.
pub fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| i64::try_from(elapsed.as_secs()).unwrap_or(0))
}

/// How long ago `then` was, as GitHub says it, with the date as its title.
pub fn ago(then: Timestamp) -> String {
    let then = i64::try_from(then.as_secs()).unwrap_or(0);
    format!(
        "<time title=\"{}\">{}</time>",
        date(then),
        relative(then, now())
    )
}

/// `then` relative to `now`. A pure function.
pub fn relative(then: i64, now: i64) -> String {
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

#[cfg(test)]
mod tests {
    use nostr::event::{EventBuilder, FinalizeEvent};
    use nostr::key::Keys;

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

    #[test]
    fn markup_in_text_is_escaped() {
        assert_eq!(escape("<script>\"&'"), "&lt;script&gt;&quot;&amp;&#39;");
    }

    #[test]
    fn the_switcher_offers_only_the_apps_that_are_on() {
        let both = Apps::new([App::Social, App::Repo]);
        let page_html = page(&both, App::Repo, "t", "", "");
        let repo = page_html.find("href=\"/repo\"").expect("repo link");
        let social = page_html.find("href=\"/social\"").expect("social link");
        assert!(repo < social, "the switcher's order is fixed");
        assert!(page_html.contains("app-link current\" href=\"/repo\""));
        assert!(page_html.contains("href=\"/repo/assets/style."));

        let alone = Apps::new([App::Repo]);
        assert!(!page(&alone, App::Repo, "t", "", "").contains("/social"));
        assert!(!both.serves(App::Social) || both.serves(App::Repo));
        assert!(!alone.serves(App::Social));
    }

    #[test]
    fn an_app_is_named_as_toon_ui_apps_names_it() {
        for app in App::ALL {
            assert_eq!(App::from_name(app.name()), Some(app));
            assert_eq!(app.prefix(), format!("/{}", app.name()));
        }
        assert_eq!(App::from_name("git"), None);
    }

    #[test]
    fn a_nip05_claim_is_kept_only_in_a_shape_a_url_can_carry() {
        assert_eq!(
            nip05_identifier(" Maya@Tide.dev "),
            Some("maya@tide.dev".to_owned())
        );
        assert_eq!(nip05_identifier("_@tide.dev"), Some("tide.dev".to_owned()));
        for bad in [
            "maya",
            "maya@localhost",
            "ma ya@tide.dev",
            "maya@tide.dev/x",
            "@tide.dev",
            "maya@.dev",
            "<b>@x.y",
        ] {
            assert_eq!(nip05_identifier(bad), None, "{bad}");
        }
    }

    #[test]
    fn the_newest_profile_gives_the_name_and_the_claim() {
        let keys = Keys::generate();
        let profile = |at: u64, content: &str| {
            EventBuilder::new(Kind::Metadata, content)
                .custom_created_at(Timestamp::from(at))
                .finalize(&keys)
                .unwrap()
        };
        let events = [
            profile(1, r#"{"name":"old","nip05":"old@x.y"}"#),
            profile(2, r#"{"display_name":" ","name":"Ada","nip05":"ada@x.y"}"#),
        ];
        let profiles = Profiles::from_events(&[keys.public_key()], &events);
        assert_eq!(
            profiles.names().get(&keys.public_key()).map(String::as_str),
            Some("Ada")
        );
        assert_eq!(profiles.nip05(&keys.public_key()), Some("ada@x.y"));
    }

    #[test]
    fn a_profile_is_a_bot_only_when_it_says_bot_true() {
        let (bot, yes, plain) = (Keys::generate(), Keys::generate(), Keys::generate());
        let profile = |keys: &Keys, content: &str| {
            EventBuilder::new(Kind::Metadata, content)
                .finalize(keys)
                .unwrap()
        };
        let keys = [bot.public_key(), yes.public_key(), plain.public_key()];
        let profiles = Profiles::from_events(
            &keys,
            &[
                profile(&bot, r#"{"name":"b","bot":true}"#),
                profile(&yes, r#"{"name":"y","bot":"yes"}"#),
                profile(&plain, r#"{"name":"p"}"#),
            ],
        );
        assert!(profiles.bots().contains(&bot.public_key()));
        assert!(!profiles.bots().contains(&yes.public_key()));
        assert!(bot_chip(profiles.bots(), &bot.public_key()).contains("chip-bot"));
        assert_eq!(bot_chip(profiles.bots(), &plain.public_key()), "");
    }

    #[test]
    fn a_person_links_to_their_profile_only_when_social_is_on() {
        let key = Keys::generate().public_key();
        let names = HashMap::from([(key, "Ada".to_owned())]);
        let path = format!("/social/p/{}", npub(&key));
        assert_eq!(profile_path(&key), path);
        let on = Apps::new([App::Repo, App::Social]);
        assert!(person(&on, &names, &key).contains(&format!("href=\"{path}\"")));
        assert!(person_avatar(&on, &names, &key, true).contains(&format!("href=\"{path}\"")));
        let off = Apps::new([App::Repo]);
        assert!(!person(&off, &names, &key).contains("href"));
        assert!(!person_avatar(&off, &names, &key, false).contains("href"));
    }

    #[test]
    fn a_tab_carries_its_count_and_marks_the_current_page() {
        let bar = tabs(
            "Repo",
            &[
                Tab::new("Stars", icon::STAR, "/s").current(true).count(3),
                Tab::new("Talk <b>", icon::COMMENT, "/t"),
            ],
        );
        assert!(bar.contains("class=\"tab active\" href=\"/s\" aria-current=\"page\""));
        assert!(bar.contains("<span class=\"count\">3</span>"));
        assert!(bar.contains("Talk &lt;b&gt;"));
    }
}
