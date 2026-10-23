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
    pages.previews.respond(&headers, card).await
}

/// `/social/og/p/<npub>.png`.
pub(crate) async fn profile(
    State(pages): State<Pages>,
    Path(file): Path<String>,
    headers: HeaderMap,
) -> Response {
    let card = profile_card(&pages, &file).await;
    pages.previews.respond(&headers, card).await
}

/// `/social/og/e/<id>.png`.
pub(crate) async fn ship(
    State(pages): State<Pages>,
    Path(file): Path<String>,
    headers: HeaderMap,
) -> Response {
    let card = ship_card(&pages, &file).await;
    pages.previews.respond(&headers, card).await
}

/// `/social/og/a/<npub>/<d>.png`.
pub(crate) async fn article(
    State(pages): State<Pages>,
    Path((author, file)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let card = article_card(&pages, &author, &file).await;
    pages.previews.respond(&headers, card).await
}

/// A repo as its page counts it: forks as the Forks tab does.
async fn repo_face(pages: &Pages, card: &RepoCard) -> ui_preview::Repo {
    let forks = Lineage::load(&pages.store, card).await.people();
    card.preview(Some(forks))
}

async fn repo_card(pages: &Pages, owner: &str, file: &str) -> Option<Card> {
    let address = RepoAddress::new(parse_pubkey(owner)?, ui_preview::stem(file)?)?;
    let card = RepoCard::load(&pages.store, &address).await?;
    let face = Face::Repo(repo_face(pages, &card).await);
    Some(Card::new(Mark::Social, pages.card_host(), face))
}

async fn profile_card(pages: &Pages, file: &str) -> Option<Card> {
    let key = parse_pubkey(ui_preview::stem(file)?)?;
    let profile = crate::profile::preview(&pages.store, key).await?;
    Some(Card::new(
        Mark::Social,
        pages.card_host(),
        Face::Profile(profile),
    ))
}

async fn ship_card(pages: &Pages, file: &str) -> Option<Card> {
    let id = parse_event_id(ui_preview::stem(file)?)?;
    let event = event_by_id(&pages.store, id).await?;
    let cards = load_cards(&pages.store, post::repos_of(&event)).await;
    let repo = ship::ship_repo(&event, |repo, key| {
        cards.get(repo).is_some_and(|card| card.is_maker(key))
    })?;
    let card = cards.get(&repo)?;
    let face = Face::Ship {
        headline: post::ship_title(&event),
        repo: repo_face(pages, card).await,
    };
    Some(Card::new(Mark::Social, pages.card_host(), face))
}

async fn article_card(pages: &Pages, author: &str, file: &str) -> Option<Card> {
    let address = ArticleAddress::new(parse_pubkey(author)?, ui_preview::stem(file)?);
    let event = article::load(&pages.store, &address).await?;
    let names: HashMap<PublicKey, String> = Profiles::load(&pages.store, &[event.pubkey])
        .await
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
    Some(Card::new(Mark::Social, pages.card_host(), face))
}
