//! `/repo`'s link preview images (ADR 0006): `/repo/og/<npub>/<d>.png`, a
//! repository's card, and `/repo/og/site.png`, the site card the other
//! pages name. A repo page names `/social`'s card instead while `/social`
//! is on ([`crate::card::Links::preview_image`]); these are what it names
//! when `/repo` is served alone. Forks are counted as the forks this relay
//! holds that name the repo in a `u` tag.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use nostr::key::PublicKey;
use nostr::nips::nip19::FromBech32;
use ui_preview::{Card, Face, Mark};

use crate::Pages;
use crate::card::RepoCard;
use crate::lineage::ForkNetwork;
use crate::repo::RepoAddress;

/// `/repo/og/site.png`.
pub(crate) async fn site(State(pages): State<Pages>, headers: HeaderMap) -> Response {
    let card = ui_preview::site(Mark::Repos, host(&pages));
    pages.previews.respond(&headers, Some(card)).await
}

/// `/repo/og/<npub>/<d>.png`.
pub(crate) async fn repo(
    State(pages): State<Pages>,
    Path((owner, file)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let card = repo_card(&pages, &owner, &file).await;
    pages.previews.respond(&headers, card).await
}

async fn repo_card(pages: &Pages, owner: &str, file: &str) -> Option<Card> {
    let owner = PublicKey::from_bech32(owner).ok()?;
    let address = RepoAddress::new(owner, ui_preview::stem(file)?)?;
    let card = RepoCard::load(&pages.store, &address).await?;
    let forks = ForkNetwork::load(&pages.store, card.announcement())
        .await
        .forks(&address)
        .len();
    Some(Card::new(
        Mark::Repos,
        host(pages),
        Face::Repo(card.preview(Some(forks))),
    ))
}

fn host(pages: &Pages) -> Option<String> {
    ui_preview::footer_host(&pages.apps, pages.defaults.relay_host.as_deref())
}
