//! The paid live feed (#215) over real sockets: the subscribe route credits a
//! key, the live feed is sold to the connection that proves it holds the key,
//! and every event is debited once.

mod common;

use axum::body::Body;
use axum::http::Request;
use base64::Engine;
use base64::engine::general_purpose::STANDARD;
use common::{Client, delivery, recorded_schema, running_with, signed, signed_by, write};
use http_body_util::BodyExt;
use nostr::event::{Event, EventBuilder, FinalizeEvent, Kind, Tag};
use nostr::key::Keys;
use nostr::types::Timestamp;
use relay::Relay;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tower::ServiceExt;

const PRICE: u64 = 1000;
const BROADCAST: u64 = 10;

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("after the epoch")
        .as_secs()
}

/// The settings of a relay that sells its feed.
fn selling(name: &str) -> Option<String> {
    match name {
        "TOON_CONNECTOR_URL" => Some("http://connector.invalid:3000/ilp".to_string()),
        "TOON_WRITE_ILP_ADDRESS" => Some("g.toon.relay".to_string()),
        "TOON_SUBSCRIBE_ILP_ADDRESS" => Some("g.toon.relay.subscribe".to_string()),
        "TOON_BROADCAST_PRICE" => Some(BROADCAST.to_string()),
        "TOON_RELAY_URL" => Some("wss://relay.example".to_string()),
        _ => None,
    }
}

fn authorization(keys: &Keys, method: &str, body: Option<&str>) -> String {
    let mut tags = vec![
        vec!["u".to_string(), "https://relay.example/".to_string()],
        vec!["method".to_string(), method.to_string()],
    ];
    if let Some(body) = body {
        let hash: String = Sha256::digest(body.as_bytes())
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        tags.push(vec!["payload".to_string(), hash]);
    }
    let event = EventBuilder::new(Kind::from(27235), "")
        .tags(tags.into_iter().map(|tag| Tag::parse(tag).expect("a tag")))
        .custom_created_at(Timestamp::from(now()))
        .finalize(keys)
        .expect("signed");
    let token = STANDARD.encode(serde_json::to_vec(&event).expect("JSON"));
    format!("Nostr {token}")
}

async fn answer(router: axum::Router, request: Request<Body>) -> (u16, Value) {
    let response = router.oneshot(request).await.expect("infallible");
    let status = response.status().as_u16();
    let body = response
        .into_body()
        .collect()
        .await
        .expect("a body")
        .to_bytes();
    (status, serde_json::from_slice(&body).unwrap_or(Value::Null))
}

/// A subscribe packet as the connector delivers it, stating `amount`.
async fn pay(relay: &Relay, keys: &Keys, body: Value, amount: Option<u64>) -> (u16, Value) {
    let body = body.to_string();
    let mut request = Request::post("/subscribe")
        .header("content-type", "application/json")
        .header("authorization", authorization(keys, "POST", Some(&body)));
    if let Some(amount) = amount {
        request = request
            .header("x-toon-amount", amount.to_string())
            .header("x-toon-payer", format!("evm:0x{}", "ab".repeat(32)))
            .header("x-toon-chain", "evm");
    }
    answer(
        relay.write_router(),
        request.body(Body::from(body)).expect("a request"),
    )
    .await
}

async fn balance(relay: &Relay, keys: &Keys) -> (u16, Value) {
    let request = Request::get("/")
        .header("accept", "application/toon-subscription+json")
        .header("authorization", authorization(keys, "GET", None))
        .body(Body::empty())
        .expect("a request");
    answer(relay.read_router(), request).await
}

/// A client that has answered the relay's challenge as `keys`.
async fn connected_as(url: &str, keys: &Keys) -> Client {
    let mut client = Client::connect(url).await;
    let challenge = client.next().await.expect("a challenge");
    assert_eq!(challenge[0], "AUTH");
    let auth = EventBuilder::new(Kind::Authentication, "")
        .tags([
            Tag::parse(["relay", "wss://relay.example"]).expect("a tag"),
            Tag::parse(["challenge", challenge[1].as_str().expect("text")]).expect("a tag"),
        ])
        .finalize(keys)
        .expect("signed");
    client.send(json!(["AUTH", auth])).await;
    let ok = client.next().await.expect("an answer");
    assert_eq!(ok, json!(["OK", auth.id, true, ""]));
    client
}

fn kind(kind: u16) -> Event {
    signed(kind, now(), &[])
}

async fn publish(relay: &Relay, event: &Event) {
    let (status, _) = write(relay, delivery(event)).await;
    assert_eq!(status, 200);
}

fn text(frame: &Value, at: usize) -> &str {
    frame[at].as_str().expect("text")
}

#[tokio::test]
async fn a_first_payment_opens_a_subscription_and_a_later_one_tops_it_up() {
    let running = running_with(selling).await;
    let keys = Keys::generate();
    let filter = json!({ "kinds": [1] });

    let (status, first) = pay(
        &running.relay,
        &keys,
        json!({ "filter": filter }),
        Some(PRICE),
    )
    .await;
    assert_eq!(status, 200);
    assert_eq!(
        first,
        json!({
            "pubkey": keys.public_key().to_hex(),
            "credited": PRICE,
            "balance": PRICE,
            "broadcast_price": BROADCAST,
            "filter": filter,
        })
    );

    // Credited what the connector says it charged, and the filter kept.
    let (_, second) = pay(&running.relay, &keys, json!({}), Some(2500)).await;
    assert_eq!(second["credited"], 2500);
    assert_eq!(second["balance"], PRICE + 2500);
    assert_eq!(second["filter"], filter);

    // A later packet may replace the filter.
    let (_, third) = pay(
        &running.relay,
        &keys,
        json!({ "filter": { "kinds": [7] } }),
        Some(1),
    )
    .await;
    assert_eq!(third["filter"], json!({ "kinds": [7] }));
    assert_eq!(third["balance"], PRICE + 2501);
}

#[tokio::test]
async fn a_packet_that_states_no_amount_credits_nothing_the_relay_cannot_name() {
    // The connector is not read in this test, so there is no route price.
    let running = running_with(selling).await;
    let (status, body) = pay(
        &running.relay,
        &Keys::generate(),
        json!({ "filter": {} }),
        None,
    )
    .await;
    assert_eq!(status, 503);
    assert_eq!(body["error"]["code"], "route_unknown");
}

#[tokio::test]
async fn a_request_that_is_refused_credits_nothing() {
    let running = running_with(selling).await;
    let keys = Keys::generate();

    // No authorization at all, a body that is not an object, a filter that is
    // not one, and a first payment with no filter.
    let request = Request::post("/subscribe")
        .body(Body::from("{}"))
        .expect("a request");
    let (status, body) = answer(running.relay.write_router(), request).await;
    assert_eq!(
        (status, &body["error"]["code"]),
        (401, &json!("unauthorized"))
    );

    for (body, status, code) in [
        (json!([]), 400, "invalid_request"),
        (json!({ "filter": [] }), 400, "invalid_request"),
        (
            json!({ "filter": { "kinds": "x" } }),
            400,
            "invalid_request",
        ),
        (json!({}), 400, "filter_required"),
    ] {
        let (got, answered) = pay(&running.relay, &keys, body.clone(), Some(PRICE)).await;
        assert_eq!(
            (got, &answered["error"]["code"]),
            (status, &json!(code)),
            "{body}"
        );
    }
    assert_eq!(balance(&running.relay, &keys).await.0, 404);

    // And once subscribed, a refused top-up leaves the balance as it was.
    pay(&running.relay, &keys, json!({ "filter": {} }), Some(PRICE)).await;
    let (status, _) = pay(&running.relay, &keys, json!({ "filter": [] }), Some(PRICE)).await;
    assert_eq!(status, 400);
    assert_eq!(balance(&running.relay, &keys).await.1["balance"], PRICE);
}

#[tokio::test]
async fn the_subscriber_reads_its_balance_and_the_operator_lists_every_one() {
    let running = running_with(selling).await;
    let (one, two) = (Keys::generate(), Keys::generate());
    pay(
        &running.relay,
        &one,
        json!({ "filter": { "kinds": [1] } }),
        Some(PRICE),
    )
    .await;
    pay(&running.relay, &two, json!({ "filter": {} }), Some(40)).await;

    let (status, body) = balance(&running.relay, &one).await;
    assert_eq!(status, 200);
    assert_eq!(
        body,
        json!({
            "pubkey": one.public_key().to_hex(),
            "balance": PRICE,
            "broadcast_price": BROADCAST,
            "filter": { "kinds": [1] },
        })
    );
    let (status, none) = balance(&running.relay, &Keys::generate()).await;
    assert_eq!(
        (status, &none["error"]["code"]),
        (404, &json!("not_subscribed"))
    );

    let (status, listed) = answer(
        running.relay.write_router(),
        Request::get("/subscribers")
            .body(Body::empty())
            .expect("a request"),
    )
    .await;
    assert_eq!(status, 200);
    let mut listed: Vec<(String, u64)> = listed["subscribers"]
        .as_array()
        .expect("a list")
        .iter()
        .map(|s| {
            (
                s["pubkey"].as_str().expect("text").to_string(),
                s["balance"].as_u64().expect("a number"),
            )
        })
        .collect();
    listed.sort();
    let mut expected = vec![
        (one.public_key().to_hex(), PRICE),
        (two.public_key().to_hex(), 40),
    ];
    expected.sort();
    assert_eq!(listed, expected);
}

#[tokio::test]
async fn a_subscriber_is_sent_what_its_filter_asks_for_and_each_event_is_debited() {
    let running = running_with(selling).await;
    let (keys, author) = (Keys::generate(), Keys::generate());
    pay(
        &running.relay,
        &keys,
        json!({ "filter": { "kinds": [1] } }),
        Some(PRICE),
    )
    .await;
    let stored = signed_by(&author, 1, now(), &[]);
    publish(&running.relay, &stored).await;

    let mut client = connected_as(&running.read_url, &keys).await;
    let found = client.req("feed", json!({ "kinds": [1] })).await;
    assert_eq!(found, vec![serde_json::to_value(&stored).expect("JSON")]);
    assert_eq!(balance(&running.relay, &keys).await.1["balance"], PRICE);

    let live = signed_by(&author, 1, now(), &[&["t", "live"]]);
    publish(&running.relay, &live).await;
    assert_eq!(client.next().await, Some(json!(["EVENT", "feed", live])));
    // Not the subscription's filter: neither sent nor debited.
    publish(&running.relay, &kind(7)).await;
    assert_eq!(client.next().await, None);
    assert_eq!(
        balance(&running.relay, &keys).await.1["balance"],
        PRICE - BROADCAST
    );
}

#[tokio::test]
async fn an_event_no_open_req_asks_for_is_not_debited() {
    let running = running_with(selling).await;
    let keys = Keys::generate();
    pay(
        &running.relay,
        &keys,
        json!({ "filter": { "kinds": [1, 7] } }),
        Some(PRICE),
    )
    .await;
    let mut client = connected_as(&running.read_url, &keys).await;
    assert!(client.req("feed", json!({ "kinds": [1] })).await.is_empty());

    publish(&running.relay, &kind(7)).await;
    assert_eq!(client.next().await, None);
    assert_eq!(balance(&running.relay, &keys).await.1["balance"], PRICE);
}

#[tokio::test]
async fn an_event_costs_one_broadcast_price_however_many_reqs_and_connections_get_it() {
    let running = running_with(selling).await;
    let keys = Keys::generate();
    pay(
        &running.relay,
        &keys,
        json!({ "filter": { "kinds": [1] } }),
        Some(PRICE),
    )
    .await;
    let mut a = connected_as(&running.read_url, &keys).await;
    let mut b = connected_as(&running.read_url, &keys).await;
    assert!(a.req("one", json!({ "kinds": [1] })).await.is_empty());
    assert!(a.req("two", json!({})).await.is_empty());
    assert!(b.req("three", json!({ "kinds": [1] })).await.is_empty());

    let live = kind(1);
    publish(&running.relay, &live).await;
    let mut seen: Vec<String> = Vec::new();
    for _ in 0..2 {
        let frame = a.next().await.expect("an event on a's REQs");
        seen.push(text(&frame, 1).to_string());
    }
    seen.sort();
    assert_eq!(seen, ["one", "two"]);
    assert_eq!(b.next().await, Some(json!(["EVENT", "three", live])));
    assert_eq!(
        balance(&running.relay, &keys).await.1["balance"],
        PRICE - BROADCAST
    );
}

#[tokio::test]
async fn the_feed_is_closed_right_after_the_last_event_the_balance_paid_for() {
    let running = running_with(selling).await;
    let keys = Keys::generate();
    pay(
        &running.relay,
        &keys,
        json!({ "filter": { "kinds": [1] } }),
        Some(2 * BROADCAST),
    )
    .await;
    let mut client = connected_as(&running.read_url, &keys).await;
    assert!(client.req("feed", json!({ "kinds": [1] })).await.is_empty());

    let events = [kind(1), kind(1), kind(1)];
    for event in &events {
        publish(&running.relay, event).await;
    }
    let mut got = Vec::new();
    for _ in 0..3 {
        got.push(client.next().await.expect("a frame"));
    }
    assert_eq!(got[0][0], "EVENT");
    assert_eq!(got[1][0], "EVENT");
    assert_eq!(
        (got[2][0].as_str(), got[2][1].as_str()),
        (Some("CLOSED"), Some("feed"))
    );
    assert!(text(&got[2], 2).starts_with("payment-required:"));
    assert_eq!(client.next().await, None, "the third event is not sent");
    assert_eq!(balance(&running.relay, &keys).await.1["balance"], 0);

    // The connection stays, and a REQ is now a free read.
    client.send(json!(["REQ", "again", { "kinds": [1] }])).await;
    let mut frames = Vec::new();
    while let Some(frame) = client.next().await {
        let done = frame[0] == "CLOSED";
        frames.push(frame);
        if done {
            break;
        }
    }
    assert_eq!(frames.last().expect("a frame")[0], "CLOSED");
    assert!(text(frames.last().expect("a frame"), 2).starts_with("payment-required:"));
}

#[tokio::test]
async fn a_balance_below_one_broadcast_price_is_exhausted_and_counts_toward_the_next_payment() {
    let running = running_with(selling).await;
    let keys = Keys::generate();
    pay(
        &running.relay,
        &keys,
        json!({ "filter": {} }),
        Some(BROADCAST - 1),
    )
    .await;
    let mut client = connected_as(&running.read_url, &keys).await;
    client.send(json!(["REQ", "feed", {}])).await;
    let eose = client.next().await.expect("EOSE");
    assert_eq!(eose[0], "EOSE");
    let closed = client.next().await.expect("CLOSED");
    assert!(text(&closed, 2).starts_with("payment-required:"));
    let (_, top) = pay(&running.relay, &keys, json!({}), Some(5)).await;
    assert_eq!(top["balance"], BROADCAST + 4);
}

#[tokio::test]
async fn a_read_without_a_subscription_is_stored_events_then_eose_then_closed() {
    let running = running_with(selling).await;
    let stored = kind(1);
    publish(&running.relay, &stored).await;

    // Not authenticated: auth-required.
    let mut client = Client::connect(&running.read_url).await;
    assert_eq!(client.next().await.expect("a challenge")[0], "AUTH");
    client.send(json!(["REQ", "q", { "kinds": [1] }])).await;
    assert_eq!(client.next().await, Some(json!(["EVENT", "q", stored])));
    assert_eq!(client.next().await, Some(json!(["EOSE", "q"])));
    let closed = client.next().await.expect("CLOSED");
    assert_eq!(closed[0], "CLOSED");
    assert!(text(&closed, 2).starts_with("auth-required:"));
    publish(&running.relay, &kind(1)).await;
    assert_eq!(client.next().await, None);

    // Authenticated with no subscription: payment-required.
    let mut client = connected_as(&running.read_url, &Keys::generate()).await;
    client.send(json!(["REQ", "q", { "kinds": [1] }])).await;
    let mut last = client.next().await.expect("a frame");
    while last[0] != "CLOSED" {
        last = client.next().await.expect("a frame");
    }
    assert!(text(&last, 2).starts_with("payment-required:"));
}

#[tokio::test]
async fn a_payment_belongs_to_the_key_that_signed_it_and_not_to_anyone_else() {
    let running = running_with(selling).await;
    let (paid, other) = (Keys::generate(), Keys::generate());
    pay(&running.relay, &paid, json!({ "filter": {} }), Some(PRICE)).await;

    let mut stranger = connected_as(&running.read_url, &other).await;
    stranger.send(json!(["REQ", "feed", {}])).await;
    let mut last = stranger.next().await.expect("a frame");
    while last[0] != "CLOSED" {
        last = stranger.next().await.expect("a frame");
    }
    publish(&running.relay, &kind(1)).await;
    assert_eq!(balance(&running.relay, &paid).await.1["balance"], PRICE);
}

#[tokio::test]
async fn a_connection_that_authenticates_as_another_key_loses_the_feed_it_had() {
    let running = running_with(selling).await;
    let keys = Keys::generate();
    pay(&running.relay, &keys, json!({ "filter": {} }), Some(PRICE)).await;
    let mut client = Client::connect(&running.read_url).await;
    let challenge = client.next().await.expect("a challenge");
    let auth = |keys: &Keys| {
        EventBuilder::new(Kind::Authentication, "")
            .tags([
                Tag::parse(["relay", "wss://relay.example"]).expect("a tag"),
                Tag::parse(["challenge", challenge[1].as_str().expect("text")]).expect("a tag"),
            ])
            .finalize(keys)
            .expect("signed")
    };
    client.send(json!(["AUTH", auth(&keys)])).await;
    client.next().await.expect("OK");
    assert!(client.req("feed", json!({})).await.is_empty());

    client.send(json!(["AUTH", auth(&Keys::generate())])).await;
    let mut frames = [
        client.next().await.expect("a frame"),
        client.next().await.expect("a frame"),
    ];
    frames.sort_by_key(|frame| frame[0].to_string());
    assert_eq!(frames[0][0], "CLOSED");
    assert!(text(&frames[0], 2).starts_with("auth-required:"));
    assert_eq!(frames[1][0], "OK");

    publish(&running.relay, &kind(1)).await;
    assert_eq!(client.next().await, None);
    assert_eq!(balance(&running.relay, &keys).await.1["balance"], PRICE);
}

#[tokio::test]
async fn an_auth_that_does_not_answer_this_connection_is_refused() {
    let running = running_with(selling).await;
    let keys = Keys::generate();
    let mut client = Client::connect(&running.read_url).await;
    let challenge = client.next().await.expect("a challenge");
    let bad = |challenge: &str, relay: &str| {
        EventBuilder::new(Kind::Authentication, "")
            .tags([
                Tag::parse(["relay", relay]).expect("a tag"),
                Tag::parse(["challenge", challenge]).expect("a tag"),
            ])
            .finalize(&keys)
            .expect("signed")
    };
    for event in [
        bad("not-the-challenge", "wss://relay.example"),
        bad(
            challenge[1].as_str().expect("text"),
            "wss://elsewhere.example",
        ),
    ] {
        client.send(json!(["AUTH", event])).await;
        let ok = client.next().await.expect("an answer");
        assert_eq!(ok[2], false);
    }
}

#[tokio::test]
async fn the_relays_own_key_and_the_keys_the_operator_names_follow_the_feed_for_nothing() {
    // The relay's own key is `11…11`.
    let own = Keys::new(nostr::key::SecretKey::from_hex(&"1".repeat(64)).expect("a key"));
    let named = Keys::generate();
    let running = running_with(|name| match name {
        "TOON_OPERATOR_PUBKEYS" => Some(named.public_key().to_hex()),
        other => selling(other),
    })
    .await;

    for (keys, kind_of_live) in [(&own, 1), (&named, 2)] {
        let mut client = connected_as(&running.read_url, keys).await;
        assert!(
            client
                .req("feed", json!({ "kinds": [kind_of_live] }))
                .await
                .is_empty()
        );
        let live = kind(kind_of_live);
        publish(&running.relay, &live).await;
        assert_eq!(client.next().await, Some(json!(["EVENT", "feed", live])));
        assert_eq!(balance(&running.relay, keys).await.0, 404);
    }
}

#[tokio::test]
async fn a_relay_that_sells_nothing_keeps_its_feed_free_and_has_no_subscribe_route() {
    let running = running_with(|_| None).await;
    let mut client = Client::connect(&running.read_url).await;
    assert!(client.req("feed", json!({ "kinds": [1] })).await.is_empty());
    let live = kind(1);
    publish(&running.relay, &live).await;
    assert_eq!(client.next().await, Some(json!(["EVENT", "feed", live])));

    let request = Request::post("/subscribe")
        .body(Body::from("{}"))
        .expect("a request");
    assert_eq!(answer(running.relay.write_router(), request).await.0, 404);
}

#[tokio::test]
async fn balances_are_kept_in_a_table_that_is_only_added_and_survive_a_restart() {
    let data = tempfile::tempdir().expect("a temp dir");
    common::typescript_database(data.path());
    let before = recorded_schema(&data.path().join("events.db"));
    let data_dir = data.path().to_string_lossy().into_owned();
    let open = || {
        let config = relay::Config::from_env(|name| match name {
            "TOON_SECRET_KEY" => Some("1".repeat(64)),
            "TOON_DATA_DIR" => Some(data_dir.clone()),
            other => selling(other),
        })
        .expect("a complete configuration");
        Relay::open(&config).expect("the database opens")
    };
    let keys = Keys::generate();
    let first = open();
    let (status, _) = pay(
        &first,
        &keys,
        json!({ "filter": { "kinds": [1] } }),
        Some(PRICE),
    )
    .await;
    assert_eq!(status, 200);
    drop(first);

    // Only added to: every statement of the TypeScript schema is still there,
    // in its place, and one more follows.
    let after = recorded_schema(&data.path().join("events.db"));
    assert_eq!(after[..before.len()], before[..]);
    assert_eq!(after.len(), before.len() + 1);
    assert!(after[before.len()].contains("feed_subscriptions"));

    let second = open();
    let (status, body) = balance(&second, &keys).await;
    assert_eq!(status, 200);
    assert_eq!(body["balance"], PRICE);
    assert_eq!(body["filter"], json!({ "kinds": [1] }));
}

#[tokio::test]
async fn a_relay_that_sells_nothing_leaves_the_database_as_the_typescript_relay_wrote_it() {
    let running = running_with(|_| None).await;
    let tables = recorded_schema(&running.database);
    assert!(tables.iter().all(|sql| !sql.contains("feed_subscriptions")));
}
