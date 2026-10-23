//! The app UIs on the read port (#276): which are on, the read-only handle
//! each is built from, and which one a request belongs to.

#[cfg(feature = "ui-repo")]
use std::collections::HashSet;
#[cfg(feature = "ui-repo")]
use std::future::Future;
use std::path::Path;
#[cfg(feature = "ui-repo")]
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::Request;
use axum::http::Method;
#[cfg(feature = "ui-repo")]
use nostr::event::Event;
#[cfg(feature = "ui-repo")]
use nostr::filter::Filter;
use relay_ui::{AppUi, Assets};
#[cfg(feature = "ui-repo")]
use relay_ui::{EventSource, ReadOnlyStore};

use crate::auth::AuthPolicy;
use crate::groups::GroupView;
use crate::{Relay, RelayError, Store};

/// The read rules of a connection that has not authenticated (NIP-42), as a
/// session applies them. A server-rendered page cannot sign in for its
/// visitor, so every app UI reads the store as such a connection would: a
/// private or closed NIP-29 group's events, a private group's metadata, the
/// kinds the operator restricts and gift wraps meant for their recipients
/// alone are left out.
#[derive(Debug, Clone, Default)]
#[cfg_attr(not(feature = "ui-repo"), allow(dead_code))]
pub(crate) struct Unauthenticated {
    /// NIP-42's policy, while it is on.
    pub(crate) auth: Option<AuthPolicy>,
    /// NIP-29's groups, while it is on.
    pub(crate) groups: Option<GroupView>,
}

#[cfg(feature = "ui-repo")]
impl Unauthenticated {
    /// Whether such a connection may be shown `event`.
    fn may_read(&self, event: &Event) -> bool {
        self.auth
            .as_ref()
            .is_none_or(|auth| auth.shows_unauthenticated(event))
            && self
                .groups
                .as_ref()
                .is_none_or(|groups| groups.may_read(event, &HashSet::new()))
    }
}

/// The store as an app UI sees it: queries, and nothing else, answered as
/// to a connection that has not authenticated.
#[cfg(feature = "ui-repo")]
struct Reading {
    store: Store,
    rules: Unauthenticated,
}

#[cfg(feature = "ui-repo")]
impl EventSource for Reading {
    fn query(&self, filter: Filter) -> Pin<Box<dyn Future<Output = Vec<Event>> + Send + '_>> {
        Box::pin(async move {
            let events = self.store.query(filter).await.unwrap_or_else(|error| {
                eprintln!("ui: query failed: {error}");
                Vec::new()
            });
            events
                .into_iter()
                .filter(|event| self.rules.may_read(event))
                .collect()
        })
    }
}

/// The read-only handle every app UI is built from: `store` under `rules`.
#[cfg(feature = "ui-repo")]
fn reading(store: &Store, rules: &Unauthenticated) -> ReadOnlyStore {
    ReadOnlyStore::new(Reading {
        store: store.clone(),
        rules: rules.clone(),
    })
}

/// The operator's default places for a browser to look for git objects
/// (`TOON_UI_ARWEAVE_GATEWAYS`, `TOON_UI_BLOB_SERVERS`). The relay only
/// writes them into pages; it never contacts them.
pub(crate) struct ObjectSources<'a> {
    pub(crate) arweave_gateways: &'a [String],
    pub(crate) blob_servers: &'a [String],
}

/// The app UIs named in `apps` (`TOON_UI_APPS`), each over a read-only handle
/// to `store` that answers under `reads`. A name this build has no app for is refused: the feature that
/// holds it was left out.
pub(crate) fn enabled(
    apps: &[String],
    store: &Store,
    assets: Option<&Assets>,
    sources: &ObjectSources<'_>,
    reads: &Unauthenticated,
) -> Result<Arc<[AppUi]>, RelayError> {
    let mut uis = Vec::new();
    for app in apps {
        match built(app, store, assets, sources, reads) {
            Some(ui) => uis.push(ui),
            None => {
                return Err(RelayError::InvalidSetting {
                    name: "TOON_UI_APPS",
                    expected: "app UIs this build includes",
                    value: app.clone(),
                });
            }
        }
    }
    Ok(uis.into())
}

/// The app UI called `name`, if this build includes it.
#[cfg_attr(not(feature = "ui-repo"), allow(unused_variables))]
fn built(
    name: &str,
    store: &Store,
    assets: Option<&Assets>,
    sources: &ObjectSources<'_>,
    reads: &Unauthenticated,
) -> Option<AppUi> {
    #[cfg(feature = "ui-repo")]
    if name == "repo" {
        return Some(ui_nip34::app(
            reading(store, reads),
            assets.cloned(),
            ui_nip34::Defaults {
                arweave_gateways: sources.arweave_gateways.to_vec(),
                blob_servers: sources.blob_servers.to_vec(),
            },
        ));
    }
    None
}

/// The built assets in `dir` (`TOON_UI_ASSETS_DIR`). Unset, or a folder
/// without a readable manifest and the files it names, is none: the relay
/// still starts and the pages that need assets say so.
pub(crate) fn assets(dir: Option<&Path>) -> Option<Assets> {
    let dir = dir?;
    let assets = Assets::load(dir);
    if assets.is_none() {
        eprintln!(
            "ui: no usable assets in {} (TOON_UI_ASSETS_DIR); browsing pages are unavailable",
            dir.display()
        );
    }
    assets
}

impl Relay {
    /// The app UI that answers `request`: a `GET` or `HEAD` under the prefix
    /// of one that is on.
    pub(crate) fn ui_for(&self, request: &Request) -> Option<&AppUi> {
        if request.method() != Method::GET && request.method() != Method::HEAD {
            return None;
        }
        self.uis.iter().find(|ui| ui.serves(request.uri().path()))
    }
}
