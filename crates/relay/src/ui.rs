//! The app UIs on the read port (#276): which are on, the read-only handle
//! each is built from, and which one a request belongs to.

#[cfg(any(feature = "ui-repo", feature = "ui-social"))]
use std::collections::HashSet;
#[cfg(any(feature = "ui-repo", feature = "ui-social"))]
use std::future::Future;
use std::path::Path;
#[cfg(any(feature = "ui-repo", feature = "ui-social"))]
use std::pin::Pin;
use std::sync::Arc;

use axum::extract::Request;
use axum::http::Method;
#[cfg(any(feature = "ui-repo", feature = "ui-social"))]
use nostr::event::Event;
#[cfg(any(feature = "ui-repo", feature = "ui-social"))]
use nostr::filter::Filter;
use relay_ui::chrome::{App, Apps};
use relay_ui::{AppUi, Assets};
#[cfg(any(feature = "ui-repo", feature = "ui-social"))]
use relay_ui::{EventSource, ReadOnlyStore};

use crate::auth::AuthPolicy;
use crate::groups::GroupView;
use crate::{Relay, RelayError, Store};

/// The read rules of a connection that has not authenticated (NIP-42), as a
/// session applies them. A server-rendered page cannot sign in for its
/// visitor, so every app UI reads the store as such a connection would: a
/// private or closed NIP-29 group's events, a private group's metadata, the
/// kinds the operator restricts and gift wraps meant for their recipients
/// alone are left out, on `/repo` as on `/social`.
#[derive(Debug, Clone, Default)]
#[cfg_attr(not(any(feature = "ui-repo", feature = "ui-social")), allow(dead_code))]
pub(crate) struct Unauthenticated {
    /// NIP-42's policy, while it is on.
    pub(crate) auth: Option<AuthPolicy>,
    /// NIP-29's groups, while it is on.
    pub(crate) groups: Option<GroupView>,
}

#[cfg(any(feature = "ui-repo", feature = "ui-social"))]
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
#[cfg(any(feature = "ui-repo", feature = "ui-social"))]
struct Reading {
    store: Store,
    rules: Unauthenticated,
}

#[cfg(any(feature = "ui-repo", feature = "ui-social"))]
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
#[cfg(any(feature = "ui-repo", feature = "ui-social"))]
fn reading(store: &Store, rules: &Unauthenticated) -> ReadOnlyStore {
    ReadOnlyStore::new(Reading {
        store: store.clone(),
        rules: rules.clone(),
    })
}

/// What the operator told the app UIs. The relay only writes these into
/// pages; it never contacts any of them.
pub(crate) struct UiSettings<'a> {
    /// Default places for a browser to look for git objects
    /// (`TOON_UI_ARWEAVE_GATEWAYS`, `TOON_UI_BLOB_SERVERS`).
    pub(crate) arweave_gateways: &'a [String],
    pub(crate) blob_servers: &'a [String],
    /// The host visitors' clients reach this relay at, for `nostr://` clone
    /// URLs (`TOON_UI_RELAY_URL`, else the paid feed's `TOON_RELAY_URL`).
    pub(crate) relay_host: Option<String>,
    /// The nsite gateway a live demo is linked through
    /// (`TOON_UI_NSITE_GATEWAY`).
    pub(crate) nsite_gateway: Option<&'a str>,
    /// The relay's key when it keeps NIP-29 groups (`TOON_NIP29_GROUPS`):
    /// the key their metadata is signed with.
    #[cfg_attr(not(feature = "ui-social"), allow(dead_code))]
    pub(crate) group_signer: Option<nostr::key::PublicKey>,
    /// What every app UI may read ([`Unauthenticated`]).
    pub(crate) reads: Unauthenticated,
}

/// The host and port (when one is given) of a URL configuration has already
/// checked: what a `nostr://` URL names as its relay hint.
pub(crate) fn relay_host(url: &str) -> Option<String> {
    let uri = url.parse::<hyper::Uri>().ok()?;
    let host = uri.host()?.to_ascii_lowercase();
    Some(match uri.port_u16() {
        Some(port) => format!("{host}:{port}"),
        None => host,
    })
}

/// The app UIs named in `apps` (`TOON_UI_APPS`), each over a read-only handle
/// to `store`. A name this build has no app for is refused: the feature that
/// holds it was left out.
pub(crate) fn enabled(
    apps: &[String],
    store: &Store,
    assets: Option<&Assets>,
    settings: &UiSettings<'_>,
) -> Result<Arc<[AppUi]>, RelayError> {
    let on = Apps::new(apps.iter().filter_map(|name| App::from_name(name)));
    let mut uis = Vec::new();
    for app in apps {
        match built(app, store, assets, settings, &on) {
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
#[cfg_attr(
    not(all(feature = "ui-repo", feature = "ui-social")),
    allow(unused_variables)
)]
fn built(
    name: &str,
    store: &Store,
    assets: Option<&Assets>,
    settings: &UiSettings<'_>,
    on: &Apps,
) -> Option<AppUi> {
    #[cfg(feature = "ui-repo")]
    if name == App::Repo.name() {
        return Some(ui_nip34::app(
            reading(store, &settings.reads),
            assets.cloned(),
            ui_nip34::Defaults {
                arweave_gateways: settings.arweave_gateways.to_vec(),
                blob_servers: settings.blob_servers.to_vec(),
                relay_host: settings.relay_host.clone(),
            },
            on.clone(),
        ));
    }
    #[cfg(feature = "ui-social")]
    if name == App::Social.name() {
        let mut social = ui_social::Settings::new(
            settings.relay_host.clone(),
            settings.nsite_gateway.map(str::to_owned),
        );
        if let Some(signer) = settings.group_signer {
            social = social.with_group_signer(signer);
        }
        return Some(ui_social::app(
            reading(store, &settings.reads),
            social,
            on.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_relay_hint_is_the_urls_host_and_any_port_it_names() {
        assert_eq!(
            relay_host("wss://Relay.Example/").as_deref(),
            Some("relay.example")
        );
        assert_eq!(
            relay_host("ws://localhost:7100").as_deref(),
            Some("localhost:7100")
        );
        assert_eq!(relay_host("not a url"), None);
    }
}
