//! `/social`'s link preview images (ADR 0006): what a page's `og:image`
//! names, so a link posted on X, Discord, Slack or Bluesky unfurls into a
//! card. Each is drawn from this relay's store, counted as the page it
//! stands for counts, by `ui_preview`:
//!
//! | Path                              | Card                                         |
//! | --------------------------------- | -------------------------------------------- |
//! | `/social/og/r/<npub>/<d>.png`     | a repo: owner, name, description, topics, stars, followers, forks |
//! | `/social/og/p/<npub>.png`         | a profile: name, NIP-05 claim, about, repos, followers, bot |
//! | `/social/og/e/<id>.png`           | a ship post: its headline over its repo      |
//! | `/social/og/a/<npub>/<d>.png`     | an article: title, author, summary          |
//! | `/social/og/site.png`             | every other page: the app's name             |
//!
//! A path naming something this relay does not hold is `404`, as is a post
//! that is not a ship post: its page names the site card instead.

use relay_ui::StoreError;
use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use nostr::event::EventId;
use nostr::key::PublicKey;
use relay_ui::chrome::{Profiles, npub, percent_encode, shown_name};
use ui_nip34::card::RepoCard;
use ui_nip34::repo::RepoAddress;
use ui_preview::{Card, Face, Mark};

use crate::article::{self, ArticleAddress};
use crate::lineage::Lineage;
use crate::{PREFIX, Pages, event_by_id, load_cards, parse_event_id, parse_pubkey, post, ship};

/// A profile's card.
pub(crate) fn profile_path(key: &PublicKey) -> String {
    format!("{PREFIX}/og/p/{}.png", npub(key))
}

/// A profile's card as its page's `og:image` names it: [`profile_path`]
/// with `?v=` and a name for what the card is drawn from, its pictures by
/// their URLs rather than their bytes, so the page fetches none. A crawler's
/// image cache (Discord's media proxy keeps an image by its URL) then sees a
/// new picture, banner or count as a new image, not one it already holds.
pub(crate) fn profile_image(
    pages: &Pages,
    key: &PublicKey,
    preview: Option<&(ui_preview::Profile, crate::profile::PictureUrls)>,
) -> String {
    let path = profile_path(key);
    match preview {
        Some((profile, urls)) => {
            let card = Card::new(pages.card_host(), Face::Profile(profile.clone()));
            let version = ui_preview::version(&format!("{}\n{urls:?}", card.key()));
            format!("{path}?v={version}")
        }
        None => path,
    }
}

/// A ship post's card.
pub(crate) fn ship_path(id: &EventId) -> String {
    format!("{PREFIX}/og/e/{}.png", id.to_hex())
}

/// An article's card.
pub(crate) fn article_path(address: &ArticleAddress) -> String {
    format!(
        "{PREFIX}/og/a/{}/{}.png",
        npub(address.author()),
        percent_encode(address.d())
    )
}

/// `/social/og/site.png`.
pub(crate) async fn site(State(pages): State<Pages>, headers: HeaderMap) -> Response {
    let card = ui_preview::site(Mark::Social, pages.card_host());
    pages.previews.respond(&headers, Some(card)).await
}

/// `/social/og/r/<npub>/<d>.png`.
pub(crate) async fn repo(
    State(pages): State<Pages>,
    Path((owner, file)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let card = repo_card(&pages, &owner, &file).await;
    pages.previews.respond_result(&headers, card).await
}

/// `/social/og/p/<npub>.png`.
pub(crate) async fn profile(
    State(pages): State<Pages>,
    Path(file): Path<String>,
    headers: HeaderMap,
) -> Response {
    let card = profile_card(&pages, &file).await;
    pages.previews.respond_result(&headers, card).await
}

/// `/social/og/e/<id>.png`.
pub(crate) async fn ship(
    State(pages): State<Pages>,
    Path(file): Path<String>,
    headers: HeaderMap,
) -> Response {
    let card = ship_card(&pages, &file).await;
    pages.previews.respond_result(&headers, card).await
}

/// `/social/og/a/<npub>/<d>.png`.
pub(crate) async fn article(
    State(pages): State<Pages>,
    Path((author, file)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let card = article_card(&pages, &author, &file).await;
    pages.previews.respond_result(&headers, card).await
}

/// A repo as its page counts it: forks as the Forks tab does.
async fn repo_face(pages: &Pages, card: &RepoCard) -> Result<ui_preview::Repo, StoreError> {
    let forks = Lineage::load(&pages.store, card).await?.people();
    Ok(card.preview(Some(forks)))
}

async fn repo_card(pages: &Pages, owner: &str, file: &str) -> Result<Option<Card>, StoreError> {
    let Some(address) = parse_pubkey(owner)
        .zip(ui_preview::stem(file))
        .and_then(|(owner, stem)| RepoAddress::new(owner, stem))
    else {
        return Ok(None);
    };
    let Some(card) = RepoCard::load(&pages.store, &address).await? else {
        return Ok(None);
    };
    let face = Face::Repo(repo_face(pages, &card).await?);
    Ok(Some(Card::new(pages.card_host(), face)))
}

async fn profile_card(pages: &Pages, file: &str) -> Result<Option<Card>, StoreError> {
    let Some(key) = ui_preview::stem(file).and_then(parse_pubkey) else {
        return Ok(None);
    };
    let Some((mut profile, urls)) = crate::profile::preview(&pages.store, key).await? else {
        return Ok(None);
    };
    // The picture and banner the kind 0 names, fetched at once (ADR 0011).
    // One that cannot be had is left out, and the card is drawn without it.
    let fetch = |url: Option<String>| async move {
        match url {
            Some(url) => pages.pictures.get(&url).await,
            None => None,
        }
    };
    (profile.picture, profile.banner) = tokio::join!(fetch(urls.picture), fetch(urls.banner));
    Ok(Some(Card::new(pages.card_host(), Face::Profile(profile))))
}

async fn ship_card(pages: &Pages, file: &str) -> Result<Option<Card>, StoreError> {
    let Some(id) = ui_preview::stem(file).and_then(parse_event_id) else {
        return Ok(None);
    };
    let Some(event) = event_by_id(&pages.store, id).await? else {
        return Ok(None);
    };
    let cards = load_cards(&pages.store, post::repos_of(&event)).await?;
    let Some(repo) = ship::ship_repo(&event, |repo, key| {
        cards.get(repo).is_some_and(|card| card.is_maker(key))
    }) else {
        return Ok(None);
    };
    let Some(card) = cards.get(&repo) else {
        return Ok(None);
    };
    let face = Face::Ship {
        headline: post::ship_title(&event),
        repo: repo_face(pages, card).await?,
    };
    Ok(Some(Card::new(pages.card_host(), face)))
}

async fn article_card(pages: &Pages, author: &str, file: &str) -> Result<Option<Card>, StoreError> {
    let Some((author, stem)) = parse_pubkey(author).zip(ui_preview::stem(file)) else {
        return Ok(None);
    };
    let address = ArticleAddress::new(author, stem);
    let Some(event) = article::load(&pages.store, &address).await? else {
        return Ok(None);
    };
    let names: HashMap<PublicKey, String> = Profiles::load(&pages.store, &[event.pubkey])
        .await?
        .into_names();
    let title = article::title_of(&event);
    let face = Face::Article(ui_preview::Article {
        title: if title.is_empty() {
            "Article".to_owned()
        } else {
            title
        },
        author: shown_name(&names, &event.pubkey),
        summary: article::summary_of(&event),
    });
    Ok(Some(Card::new(pages.card_host(), face)))
}
