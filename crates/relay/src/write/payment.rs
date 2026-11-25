//! The payment the connector states on a paid write. It exists so that "no
//! path but the paid-write handler can claim a payment" is a fact about types
//! rather than about how the handlers happen to be written (#185, invariant 2).
//!
//! A terminating connector states three headers on a delivery whose payment
//! it verified at its own client edge (connector ADR 0040): who paid, what
//! it charged, and on which chain. The relay does not and cannot check any of
//! it: it holds no chain state and speaks no ILP, so the connector's
//! statement is the whole trust model. What this module adds is that a
//! statement is all three headers, well-formed, or it is nothing. A fourth,
//! `X-TOON-Network`, rides along when stated (connector ADR 0086).
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
//! The header names are defined here and nowhere else in the relay
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
/// The network within that chain, when the connector names one (ADR 0086).
const NETWORK_HEADER: &str = "X-TOON-Network";

/// A payment the terminating connector states it verified. Serialized, it is
/// the `payment` object of a `200` from `POST /write`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PaymentStatement {
    payer: String,
    amount: String,
    chain: Chain,
    #[serde(skip_serializing_if = "Option::is_none")]
    network: Option<String>,
}

/// The chain a payment settles on, as the connector names it: `evm`,
/// `solana`, `xrpl`, `mina`, or any namespace a later connector adds. The
/// relay never validates a payment (ADR 0040), so a chain it has not heard of
/// is kept as stated.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct Chain(String);

impl Chain {
    /// A chain as a test names it, without reading headers.
    #[cfg(test)]
    pub(super) fn named(namespace: &str) -> Self {
        Self(namespace.to_string())
    }

    /// The chain's namespace, as stated.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for Chain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl PaymentStatement {
    /// Read the connector's statement off the headers of a paid write.
    ///
    /// `None` unless the payer, amount and chain are stated once each and are
    /// well-formed: a `namespace:key` payer (a channel key, or a claim key
    /// such as `zk-note:<allowance id>`), base-unit digits, and a lowercase
    /// chain token. For the chains whose key shape the relay has always
    /// checked, `evm` and `solana`, the payer must also be that chain's key.
    /// `X-TOON-Network` is kept when stated once and well-formed; stated
    /// badly, it is dropped without discarding the rest. Anything less than a
    /// whole statement is discarded: half a statement would be recorded and
    /// echoed as if it were all of one.
    pub(super) fn stated_on(headers: &HeaderMap) -> Option<Self> {
        let payer = stated_once(headers, PAYER_HEADER)?;
        let amount = stated_once(headers, AMOUNT_HEADER)?;
        let chain = stated_once(headers, CHAIN_HEADER)?;
        let well_formed = is_base_units(amount)
            && is_namespace(chain)
            && match chain {
                "evm" => is_evm_channel_key(payer),
                "solana" => is_solana_channel_key(payer),
                _ => is_namespaced_key(payer),
            };
        let network = stated_once(headers, NETWORK_HEADER)
            .filter(|network| network.bytes().all(|byte| byte.is_ascii_graphic()))
            .map(str::to_string);
        well_formed.then(|| Self {
            payer: payer.to_string(),
            amount: amount.to_string(),
            chain: Chain(chain.to_string()),
            network,
        })
    }

    /// The payer, namespaced: a channel key such as `evm:0x<64 hex>` or
    /// `solana:<base58>`, or the claim key of an allowance.
    pub fn payer(&self) -> &str {
        &self.payer
    }

    /// What the connector charged, in base units: decimal digits, as stated.
    /// It is kept as text because it is echoed as it arrived and no
    /// arithmetic is done on it here.
    pub fn amount(&self) -> &str {
        &self.amount
    }

    /// The chain the connector stated.
    pub fn chain(&self) -> &Chain {
        &self.chain
    }

    /// The network within the chain, when the connector stated one.
    pub fn network(&self) -> Option<&str> {
        self.network.as_deref()
    }
}

/// A lowercase namespace token: `[a-z0-9-]+`, starting with a letter.
fn is_namespace(token: &str) -> bool {
    token
        .bytes()
        .next()
        .is_some_and(|byte| byte.is_ascii_lowercase())
        && token
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'-'))
}

/// `namespace:key`, with a non-empty key of printable ASCII.
fn is_namespaced_key(payer: &str) -> bool {
    payer.split_once(':').is_some_and(|(namespace, key)| {
        is_namespace(namespace)
            && !key.is_empty()
            && key.bytes().all(|byte| byte.is_ascii_graphic())
    })
}

/// What the connector says it charged for a delivery, in base units, when it
/// says so: the one header of the statement a route that is not the paid
/// write needs, and read with none of the rest. A subscribe packet credits
/// what its route charged and never the payer (#215), so the payer and the
/// chain are not read at all. Anything but one well-formed amount is `None`.
pub(crate) fn charged_amount(headers: &HeaderMap) -> Option<u64> {
    let amount = stated_once(headers, AMOUNT_HEADER)?;
    if is_base_units(amount) {
        amount.parse().ok()
    } else {
        None
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
