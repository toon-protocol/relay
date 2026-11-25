//! `/repo`'s link preview images (ADR 0006): `/repo/og/<npub>/<d>.png`, a
//! repository's card, and `/repo/og/site.png`, the site card the other
//! pages name. A repo page names `/social`'s card instead while `/social`
//! is on ([`crate::card::Links::preview_image`]); these are what it names
//! when `/repo` is served alone. Forks are counted by the repo engagement
//! module ([`crate::engagement`]), as every page counts them.

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::response::Response;
use nostr::key::PublicKey;
use nostr::nips::nip19::FromBech32;
use relay_ui::{ReadOnlyStore, StoreError};
use ui_preview::{Card, Face, Mark};

use crate::Pages;
use crate::card::RepoCard;
use crate::engagement::{self, Act};
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
    pages.previews.respond_result(&headers, card).await
}

async fn repo_card(pages: &Pages, owner: &str, file: &str) -> Result<Option<Card>, StoreError> {
    let Ok(owner) = PublicKey::from_bech32(owner) else {
        return Ok(None);
    };
    let Some(address) = ui_preview::stem(file).and_then(|stem| RepoAddress::new(owner, stem))
    else {
        return Ok(None);
    };
    let Some(card) = RepoCard::load(&pages.store, &address).await? else {
        return Ok(None);
    };
    let face = face_of(&pages.store, &card).await?;
    Ok(Some(Card::new(host(pages), Face::Repo(face))))
}

/// A repo as its page counts it: forks from the repo engagement module.
async fn face_of(store: &ReadOnlyStore, card: &RepoCard) -> Result<ui_preview::Repo, StoreError> {
    let forks = engagement::load_one(store, card.address(), &[Act::Fork])
        .await?
        .fork_count();
    Ok(card.preview(Some(forks)))
}

fn host(pages: &Pages) -> Option<String> {
    ui_preview::footer_host(&pages.apps, pages.defaults.relay_host.as_deref())
}

#[cfg(test)]
mod tests {
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::Arc;

    use nostr::event::{Event, EventBuilder, FinalizeEvent, Kind, Tag};
    use nostr::filter::{Filter, MatchEventOptions};
    use nostr::key::Keys;
    use relay_ui::EventSource;

    use super::*;

    struct Held(Arc<Vec<Event>>);

    impl EventSource for Held {
        fn query(
            &self,
            filter: Filter,
        ) -> Pin<Box<dyn Future<Output = Result<Vec<Event>, StoreError>> + Send + '_>> {
            let found = self
                .0
                .iter()
                .filter(|event| filter.match_event(event, MatchEventOptions::new()))
                .cloned()
                .collect();
            Box::pin(async move { Ok(found) })
        }
    }

    fn announce(keys: &Keys, d: &str, tags: &[&[&str]]) -> Event {
        EventBuilder::new(Kind::from(30_617), "")
            .tags(
                [&["d", d][..]]
                    .into_iter()
                    .chain(tags.iter().copied())
                    .map(|tag| Tag::parse(tag.iter().copied()).unwrap()),
            )
            .finalize(keys)
            .unwrap()
    }

    /// One repo, two forks (a bare coordinate and `|<git url>`) and a copy
    /// that only shares the earliest unique commit: the card says 2 forks.
    #[tokio::test]
    async fn the_preview_card_counts_distinct_fork_repos() {
        let (owner, bare, url, copier) = (
            Keys::generate(),
            Keys::generate(),
            Keys::generate(),
            Keys::generate(),
        );
        let commit = "b".repeat(40);
        let parent = announce(&owner, "r", &[&["r", &commit, "euc"]]);
        let coordinate = RepoAddress::of(&parent).unwrap().coordinate();
        let store = ReadOnlyStore::new(Held(Arc::new(vec![
            parent.clone(),
            announce(&bare, "r", &[&["u", &coordinate]]),
            announce(
                &url,
                "r",
                &[&["u", &format!("{coordinate}|https://example.com/r.git")]],
            ),
            announce(&copier, "copy", &[&["r", &commit, "euc"]]),
        ])));
        let card = RepoCard::from_announcement(&parent).unwrap();
        assert_eq!(face_of(&store, &card).await.unwrap().forks, Some(2));
    }
}
