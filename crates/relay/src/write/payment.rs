//! The payment the connector states on a paid write. It exists so that "no
//! path but the paid-write handler can claim a payment" is a fact about types
//! rather than about how the handlers happen to be written (#185, invariant 2).
//!
//! A terminating connector states three headers on a delivery whose payment
//! it verified at its own client edge (connector ADR 0040): who paid, what
//! it charged, and on which chain. The relay does not and cannot check any of
//! it: it holds no chain state and speaks no ILP, so the connector's
//! statement is the whole trust model. What this module adds is that a
//! statement is all three headers, well-formed and agreeing with each other,
//! or it is nothing.
//!
//! **Absence is not "unpaid".** The headers are there only when this
//! connector took the payment and the route's price is not zero. A longer
//! path, a forwarded packet and a free route state nothing, so a missing or
//! malformed statement is `None` and never a reason to refuse the write.
//!
//! [`PaymentStatement::stated_on`] is the only constructor and is visible to
//! this module's parent alone, which is the paid-write handler: the fields
//! are private and the type is not `Deserialize`. The ephemeral lane is free
//! and has no payment to state, so it must not be built inside `write`.
//! `tests/compile_fail/` shows the other ways in failing to build.
//!
//! The three header names are defined here and nowhere else in the relay
//! (`deploy/rust-workspace.test.ts`). The connector's own definition lives in
//! a crate too heavy to depend on (#185).

use axum::http::HeaderMap;
use serde::Serialize;

/// The client channel key whose covering claim the connector verified.
const PAYER_HEADER: &str = "X-TOON-Payer";
/// What the connector charged for this delivery, in base units.
const AMOUNT_HEADER: &str = "X-TOON-Amount";
/// The chain the paying channel lives on.
const CHAIN_HEADER: &str = "X-TOON-Chain";

/// A payment the terminating connector states it verified. Serialized, it is
/// the `payment` object of a `200` from `POST /write`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaymentStatement {
    payer: String,
    amount: String,
    chain: Chain,
}

/// The chain a paying channel lives on: the namespace of its key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Chain {
    /// An `evm:` channel key.
    Evm,
    /// A `solana:` channel key.
    Solana,
}

impl std::fmt::Display for Chain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Evm => "evm",
            Self::Solana => "solana",
        })
    }
}

impl PaymentStatement {
    /// Read the connector's statement off the headers of a paid write.
    ///
    /// `None` unless all three headers are stated once each, each is
    /// well-formed, and the payer's namespace is the chain stated beside it.
    /// Anything less is discarded whole: half a statement would be recorded
    /// and echoed as if it were all of one.
    pub(super) fn stated_on(headers: &HeaderMap) -> Option<Self> {
        let payer = stated_once(headers, PAYER_HEADER)?;
        let amount = stated_once(headers, AMOUNT_HEADER)?;
        let chain = match stated_once(headers, CHAIN_HEADER)? {
            "evm" => Chain::Evm,
            "solana" => Chain::Solana,
            _ => return None,
        };
        let well_formed = is_base_units(amount)
            && match chain {
                Chain::Evm => is_evm_channel_key(payer),
                Chain::Solana => is_solana_channel_key(payer),
            };
        well_formed.then(|| Self {
            payer: payer.to_string(),
            amount: amount.to_string(),
            chain,
        })
    }

    /// The client channel key, namespaced: `evm:0x<64 hex>` or
    /// `solana:<base58>`.
    pub fn payer(&self) -> &str {
        &self.payer
    }

    /// What the connector charged, in base units: decimal digits, as stated.
    /// It is kept as text because it is echoed as it arrived and no
    /// arithmetic is done on it here.
    pub fn amount(&self) -> &str {
        &self.amount
    }

    /// The chain the payer's key is namespaced to.
    pub fn chain(&self) -> Chain {
        self.chain
    }
}

/// The value of the header `name` when it is stated exactly once and is not
/// empty. A header that arrives twice is two statements, and choosing one of
/// them would be the relay deciding what the connector said.
fn stated_once<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut values = headers.get_all(name).iter();
    let value = values.next()?.to_str().ok()?;
    (values.next().is_none() && !value.is_empty()).then_some(value)
}

/// A whole number of base units: ASCII digits, with no sign, exponent or
/// separator.
fn is_base_units(amount: &str) -> bool {
    !amount.is_empty() && amount.bytes().all(|byte| byte.is_ascii_digit())
}

/// `evm:0x` and exactly 64 lower-case hex characters.
fn is_evm_channel_key(payer: &str) -> bool {
    payer.strip_prefix("evm:0x").is_some_and(|key| {
        key.len() == 64
            && key
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    })
}

/// `solana:` and a base58 public key of 32 to 44 characters. The base58
/// alphabet has no `0`, `O`, `I` or `l`.
fn is_solana_channel_key(payer: &str) -> bool {
    payer.strip_prefix("solana:").is_some_and(|key| {
        (32..=44).contains(&key.len())
            && key.bytes().all(|byte| {
                matches!(byte, b'1'..=b'9' | b'A'..=b'H' | b'J'..=b'N' | b'P'..=b'Z' | b'a'..=b'k' | b'm'..=b'z')
            })
    })
}
