//! `POST /write`: what the connector is told about the event it delivered.
//! Driven through the router, with no socket.

mod common;

use std::time::{SystemTime, UNIX_EPOCH};

use common::{delivery, running, signed, write, write_stating};
use serde_json::json;

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("the test host's clock is after 1970")
        .as_secs()
}

#[tokio::test]
async fn a_signed_regular_event_is_answered_200_with_its_id_and_a_stored_at_time() {
    let running = running().await;
    let event = signed(1, 1_700_000_000, &[]);
    let before = now();

    let (status, body) = write(&running.relay, delivery(&event)).await;

    assert_eq!(status, 200);
    let object = body.as_object().expect("the body is an object");
    let mut keys: Vec<_> = object.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["eventId", "storedAt"]);
    assert_eq!(body["eventId"], event.id.to_hex());
    let stored_at = body["storedAt"].as_u64().expect("whole seconds");
    assert!((before..=now()).contains(&stored_at));
}

#[tokio::test]
async fn a_body_that_is_not_json_or_carries_no_event_is_400() {
    let running = running().await;
    let bodies = [
        "not json",
        "{}",
        "[]",
        r#"{"event":null}"#,
        r#"{"event":false}"#,
        r#"{"event":0}"#,
        r#"{"event":""}"#,
    ];
    for body in bodies {
        let (status, answer) = write(&running.relay, body).await;
        assert_eq!(status, 400, "{body}");
        assert!(answer["error"].is_string(), "{body}");
    }
}

#[tokio::test]
async fn a_bad_signature_or_a_tampered_event_is_422() {
    let running = running().await;
    let event = serde_json::to_value(signed(1, 1_700_000_000, &[])).expect("an event is JSON");

    let mut zero_signature = event.clone();
    zero_signature["sig"] = json!("0".repeat(128));
    let mut tampered = event.clone();
    tampered["content"] = json!("tampered after signing");
    let not_an_event = json!({ "id": "abc" });

    for bad in [zero_signature, tampered, not_an_event] {
        let (status, answer) = write(&running.relay, json!({ "event": bad }).to_string()).await;
        assert_eq!(status, 422, "{bad}");
        assert!(answer["error"].is_string(), "{bad}");
    }
}

#[tokio::test]
async fn every_kind_has_a_storage_rule_so_none_is_refused_as_not_built() {
    let running = running().await;

    // Replaceable, deletion, addressable and ephemeral: each was a 501 until
    // its rule was built.
    for kind in [0, 5, 30_000, 20_000] {
        let event = signed(kind, 1_700_000_000, &[]);
        let (status, answer) = write(&running.relay, delivery(&event)).await;
        assert_eq!(status, 200, "kind {kind}: {answer}");
    }
}

const EVM_PAYER: &str = "evm:0xabababababababababababababababababababababababababababababababab";

#[tokio::test]
async fn the_payment_the_connector_states_is_echoed_beside_the_event_id() {
    let running = running().await;
    let event = signed(1, 1_700_000_000, &[]);
    let stated = [
        ("X-TOON-Payer", EVM_PAYER),
        ("X-TOON-Amount", "1000"),
        ("X-TOON-Chain", "evm"),
    ];

    let (status, body) = write_stating(&running.relay, delivery(&event), &stated).await;

    assert_eq!(status, 200);
    assert_eq!(body["eventId"], event.id.to_hex());
    assert_eq!(
        body["payment"],
        json!({ "payer": EVM_PAYER, "amount": "1000", "chain": "evm" })
    );
}

#[tokio::test]
async fn a_solana_payment_is_echoed_as_stated() {
    let running = running().await;
    let payer = format!("solana:{}", "1".repeat(32));
    let stated = [
        ("X-TOON-Payer", payer.as_str()),
        ("X-TOON-Amount", "5"),
        ("X-TOON-Chain", "solana"),
    ];

    let (status, body) = write_stating(&running.relay, delivery(&signed(1, 1, &[])), &stated).await;

    assert_eq!(status, 200);
    assert_eq!(
        body["payment"],
        json!({ "payer": payer, "amount": "5", "chain": "solana" })
    );
}

#[tokio::test]
async fn a_statement_that_is_not_whole_and_well_formed_is_discarded_and_the_write_succeeds() {
    let running = running().await;
    let upper_case_key = format!("evm:0x{}", "AB".repeat(32));
    let short_solana_key = format!("solana:{}", "1".repeat(31));
    let long_solana_key = format!("solana:{}", "1".repeat(45));
    // `0`, `O`, `I` and `l` are not in the base58 alphabet.
    let not_base58 = format!("solana:{}", "0".repeat(32));
    let solana_key = format!("solana:{}", "1".repeat(32));
    let discarded: &[(&str, &[(&str, &str)])] = &[
        ("nothing stated", &[]),
        (
            "no payer",
            &[("X-TOON-Amount", "1000"), ("X-TOON-Chain", "evm")],
        ),
        (
            "no amount",
            &[("X-TOON-Payer", EVM_PAYER), ("X-TOON-Chain", "evm")],
        ),
        (
            "no chain",
            &[("X-TOON-Payer", EVM_PAYER), ("X-TOON-Amount", "1000")],
        ),
        (
            "an empty payer",
            &[
                ("X-TOON-Payer", ""),
                ("X-TOON-Amount", "1000"),
                ("X-TOON-Chain", "evm"),
            ],
        ),
        (
            "an empty amount",
            &[
                ("X-TOON-Payer", EVM_PAYER),
                ("X-TOON-Amount", ""),
                ("X-TOON-Chain", "evm"),
            ],
        ),
        (
            "an unknown chain",
            &[
                ("X-TOON-Payer", EVM_PAYER),
                ("X-TOON-Amount", "1000"),
                ("X-TOON-Chain", "bitcoin"),
            ],
        ),
        (
            "a chain in another case",
            &[
                ("X-TOON-Payer", EVM_PAYER),
                ("X-TOON-Amount", "1000"),
                ("X-TOON-Chain", "EVM"),
            ],
        ),
        (
            "a fractional amount",
            &[
                ("X-TOON-Payer", EVM_PAYER),
                ("X-TOON-Amount", "10.5"),
                ("X-TOON-Chain", "evm"),
            ],
        ),
        (
            "a signed amount",
            &[
                ("X-TOON-Payer", EVM_PAYER),
                ("X-TOON-Amount", "+1000"),
                ("X-TOON-Chain", "evm"),
            ],
        ),
        (
            "an amount in digits that are not ASCII",
            &[
                ("X-TOON-Payer", EVM_PAYER),
                ("X-TOON-Amount", "١٠٠٠"),
                ("X-TOON-Chain", "evm"),
            ],
        ),
        (
            "a short EVM key",
            &[
                ("X-TOON-Payer", "evm:0x1234"),
                ("X-TOON-Amount", "1000"),
                ("X-TOON-Chain", "evm"),
            ],
        ),
        (
            "an EVM key in upper case",
            &[
                ("X-TOON-Payer", &upper_case_key),
                ("X-TOON-Amount", "1000"),
                ("X-TOON-Chain", "evm"),
            ],
        ),
        (
            "an EVM payer on a Solana chain",
            &[
                ("X-TOON-Payer", EVM_PAYER),
                ("X-TOON-Amount", "1000"),
                ("X-TOON-Chain", "solana"),
            ],
        ),
        (
            "a Solana payer on an EVM chain",
            &[
                ("X-TOON-Payer", &solana_key),
                ("X-TOON-Amount", "1000"),
                ("X-TOON-Chain", "evm"),
            ],
        ),
        (
            "a Solana key too short",
            &[
                ("X-TOON-Payer", &short_solana_key),
                ("X-TOON-Amount", "1000"),
                ("X-TOON-Chain", "solana"),
            ],
        ),
        (
            "a Solana key too long",
            &[
                ("X-TOON-Payer", &long_solana_key),
                ("X-TOON-Amount", "1000"),
                ("X-TOON-Chain", "solana"),
            ],
        ),
        (
            "a Solana key that is not base58",
            &[
                ("X-TOON-Payer", &not_base58),
                ("X-TOON-Amount", "1000"),
                ("X-TOON-Chain", "solana"),
            ],
        ),
        (
            "a header stated twice",
            &[
                ("X-TOON-Payer", EVM_PAYER),
                ("X-TOON-Amount", "1000"),
                ("X-TOON-Amount", "1000"),
                ("X-TOON-Chain", "evm"),
            ],
        ),
    ];

    for (name, headers) in discarded {
        let event = signed(1, 1_700_000_000, &[]);
        let (status, body) = write_stating(&running.relay, delivery(&event), headers).await;
        assert_eq!(status, 200, "{name}");
        assert_eq!(body["eventId"], event.id.to_hex(), "{name}");
        assert!(body.get("payment").is_none(), "{name}: {body}");
    }
}
