//! The `/social` article reader and snippet page over events written
//! through `POST /write`, driven through the router with no socket: an
//! article with its series, highlights, reactions and comments, a snippet,
//! the repo page's Writing tab, and the links that lead to them.
#![cfg(all(feature = "ui-repo", feature = "ui-social"))]

mod common;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use common::{delivery, running_with, write};
use http_body_util::BodyExt;
use nostr::event::{Event, EventBuilder, FinalizeEvent, Kind, Tag};
use nostr::key::Keys;
use nostr::nips::nip01::Coordinate;
use nostr::nips::nip19::{Nip19Coordinate, ToBech32};
use nostr::types::{RelayUrl, Timestamp};
use tower::ServiceExt;

struct Page {
    status: StatusCode,
    location: Option<String>,
    text: String,
}

async fn get(relay: &relay::Relay, path: &str) -> Page {
    let response = relay
        .read_router()
        .oneshot(Request::get(path).body(Body::empty()).expect("a request"))
        .await
        .expect("infallible");
    let status = response.status();
    let location = response
        .headers()
        .get("location")
        .map(|value| value.to_str().expect("text").to_owned());
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    Page {
        status,
        location,
        text: String::from_utf8_lossy(&bytes).into_owned(),
    }
}

fn event(keys: &Keys, kind: u16, at: u64, content: &str, tags: &[&[&str]]) -> Event {
    EventBuilder::new(Kind::from(kind), content)
        .tags(
            tags.iter()
                .map(|tag| Tag::parse(tag.iter().copied()).expect("a tag")),
        )
        .custom_created_at(Timestamp::from(at))
        .finalize(keys)
        .expect("signs")
}

async fn store(relay: &relay::Relay, event: &Event) {
    assert_eq!(write(relay, delivery(event)).await.0, 200);
}

fn npub(keys: &Keys) -> String {
    keys.public_key().to_bech32().expect("encodes")
}

fn naddr(keys: &Keys, d: &str) -> String {
    Nip19Coordinate::new(
        Coordinate::new(Kind::from(30_023), keys.public_key()).identifier(d),
        Vec::<RelayUrl>::new(),
    )
    .to_bech32()
    .expect("encodes")
}

#[tokio::test]
async fn an_article_is_read_with_its_series_highlights_and_comments() {
    let running = running_with(|_| None).await;
    let relay = &running.relay;
    let maya = Keys::generate();
    let readers: Vec<Keys> = (0..4).map(|_| Keys::generate()).collect();
    let now = Timestamp::now().as_secs();
    let hex = maya.public_key().to_hex();
    let repo = format!("30617:{hex}:tidepool");
    let article_a = |d: &str| format!("30023:{hex}:{d}");

    store(
        relay,
        &event(&maya, 0, now - 100, r#"{"name":"Maya"}"#, &[]),
    )
    .await;
    store(
        relay,
        &event(
            &maya,
            30_617,
            now - 90,
            "",
            &[&["d", "tidepool"], &["name", "tidepool"]],
        ),
    )
    .await;
    let snippet = event(
        &maya,
        1_337,
        now - 85,
        "const cache = await caches.open(`h-${hash}`); // <ok>",
        &[
            &["l", "typescript"],
            &["name", "sw.ts"],
            &["description", "Name the cache after the blob"],
            &["license", "MIT"],
            &["repo", &repo],
        ],
    );
    store(relay, &snippet).await;
    let snippet_note = snippet.id.to_bech32().expect("encodes");
    store(
        relay,
        &event(
            &maya,
            30_023,
            now - 80,
            "First.",
            &[&["d", "why"], &["title", "Why I built a tide app"]],
        ),
    )
    .await;
    // An old version, replaced by the next.
    store(
        relay,
        &event(
            &maya,
            30_023,
            now - 75,
            "old text",
            &[&["d", "nsite"], &["title", "Old title"]],
        ),
    )
    .await;
    let passage = "If the hash changes, the cache name changes.";
    let body = format!(
        "An nsite is a list of hashes.\n\n## The fix\n\nName the cache after the blob. {passage} \
         Ask nostr:{}\n\nnostr:{snippet_note}\n\n<script>alert(1)</script>\n\n`{passage}`\n",
        npub(&readers[0])
    );
    let article = event(
        &maya,
        30_023,
        now - 70,
        &body,
        &[
            &["d", "nsite"],
            &["title", "Moving tidepool to an nsite"],
            &["summary", "What broke."],
            &["image", "https://cdn.test/cover.jpg"],
            &["published_at", &(now - 3_000).to_string()],
            &["a", &repo],
            &["t", "nsite"],
        ],
    );
    store(relay, &article).await;
    store(
        relay,
        &event(
            &maya,
            30_004,
            now - 60,
            "",
            &[
                &["d", "devlog"],
                &["title", "tidepool devlog"],
                &["a", &article_a("why")],
                &["a", &article_a("nsite")],
                &["a", &article_a("next")],
            ],
        ),
    )
    .await;
    // Three people highlight the passage (one twice), one another passage.
    let coordinate = article_a("nsite");
    for (who, text) in [
        (0, passage),
        (1, passage),
        (1, passage),
        (2, passage),
        (3, "An nsite is a list of hashes."),
    ] {
        store(
            relay,
            &event(&readers[who], 9_802, now - 50, text, &[&["a", &coordinate]]),
        )
        .await;
    }
    for (who, content) in [(0, "+"), (1, "+"), (1, "+"), (2, "🔥")] {
        store(
            relay,
            &event(
                &readers[who],
                7,
                now - 40,
                content,
                &[
                    &["a", &coordinate],
                    &["e", &article.id.to_hex()],
                    &["k", "30023"],
                ],
            ),
        )
        .await;
    }
    store(
        relay,
        &event(
            &readers[3],
            16,
            now - 35,
            "",
            &[&["a", &coordinate], &["k", "30023"]],
        ),
    )
    .await;
    let comment = event(
        &readers[2],
        1_111,
        now - 30,
        "Great write-up",
        &[&["A", &coordinate], &["K", "30023"], &["a", &coordinate]],
    );
    store(relay, &comment).await;

    let path = format!("/social/a/{}", naddr(&maya, "nsite"));
    let page = get(relay, &path).await;
    assert_eq!(page.status, StatusCode::OK);
    let text = &page.text;
    assert!(text.contains(&format!("data-id=\"{}\"", article.id.to_hex())));
    assert!(text.contains("Moving tidepool to an nsite"));
    assert!(!text.contains("Old title"), "the newest version is read");
    assert!(text.contains("<img class=\"cover\" src=\"https://cdn.test/cover.jpg\""));
    assert!(text.contains("Part 2 of 3"));
    assert!(text.contains("1 min read"));
    assert!(text.contains("<h2>The fix</h2>"));
    assert!(!text.contains("<script"), "raw HTML is text");
    assert!(
        text.contains("devlog of"),
        "the maker's article is a devlog"
    );
    // The series: in order, current marked, the missing part unlinked.
    assert!(text.contains("tidepool devlog"));
    assert!(text.contains("aria-current=\"page\""));
    assert!(text.contains("Why I built a tide app"));
    assert!(text.contains("next (not on this relay)"));
    // The most-highlighted passage is marked once, in prose not in code.
    assert!(text.contains(&format!(
        "<mark class=\"hl\" title=\"3 people highlighted this\">{}</mark>",
        passage
    )));
    assert_eq!(text.matches("<mark").count(), 1);
    assert!(text.contains("2 passages highlighted by 4 people"));
    // A mention, and the snippet embedded where the paragraph names it.
    assert!(text.contains(&format!("href=\"/social/p/{}\"", npub(&readers[0]))));
    assert!(text.contains(&format!("data-id=\"{}\"", snippet.id.to_hex())));
    assert!(text.contains("&lt;ok&gt;"));
    // Counts are of people.
    assert!(text.contains("+ 2</span>"), "{text}");
    assert!(text.contains("1 person boosted"));
    assert!(text.contains("1 comment from 1 person"));
    assert!(text.contains(&format!("data-id=\"{}\"", comment.id.to_hex())));

    // The other spelling, and the post page, lead to the same reader.
    let by_key = get(relay, &format!("/social/a/{}/nsite", npub(&maya))).await;
    assert_eq!(by_key.status, StatusCode::OK);
    assert!(by_key.text.contains("Moving tidepool to an nsite"));
    let redirect = get(relay, &format!("/social/e/{}", article.id.to_hex())).await;
    assert_eq!(redirect.status, StatusCode::SEE_OTHER);
    assert_eq!(redirect.location.as_deref(), Some(path.as_str()));

    // The feed and the repo page link to the reader.
    let feed = get(relay, "/social").await;
    assert!(feed.text.contains(&format!("href=\"{path}\"")));
    let base = format!("/social/r/{}/tidepool", npub(&maya));
    let maker = get(relay, &base).await;
    assert!(maker.text.contains(&format!("href=\"{path}\"")));
    let writing = get(relay, &format!("{base}/writing")).await;
    assert_eq!(writing.status, StatusCode::OK);
    assert!(writing.text.contains(&format!("href=\"{path}\"")));
    assert!(writing.text.contains("chip-maker"));

    // The snippet's page.
    let snippet_path = format!("/social/s/{}", snippet.id.to_hex());
    let page = get(relay, &snippet_path).await;
    assert_eq!(page.status, StatusCode::OK);
    assert!(page.text.contains("sw.ts"));
    assert!(page.text.contains(">typescript<"));
    assert!(page.text.contains(">MIT<"));
    assert!(page.text.contains("&lt;ok&gt;"));
    assert!(!page.text.contains("<ok>"));
    assert!(page.text.contains("class=\"repo-card"), "its repo unfurls");
    let snippets = get(relay, &format!("{base}/snippets")).await;
    assert!(snippets.text.contains(&format!("href=\"{snippet_path}\"")));
    let redirect = get(relay, &format!("/social/e/{snippet_note}")).await;
    assert_eq!(redirect.location.as_deref(), Some(snippet_path.as_str()));

    for missing in [
        format!("/social/a/{}", naddr(&maya, "none")),
        "/social/a/naddr1nope".to_owned(),
        format!("/social/s/{}", article.id.to_hex()),
        "/social/s/nope".to_owned(),
    ] {
        assert_eq!(
            get(relay, &missing).await.status,
            StatusCode::NOT_FOUND,
            "{missing}"
        );
    }
}
