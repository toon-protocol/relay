//! What the relay is told at startup. A setting that is present but wrong
//! stops the relay with a named error rather than falling back to a default
//! (#185, story 48).
//!
//! Only the settings this slice acts on are read. The rest of the TypeScript
//! relay's environment and its flags arrive with the surfaces they configure.

use nostr::key::{Keys, PublicKey, SecretKey};

use crate::RelayError;

/// The identity variables, in precedence order: `NOSTR_SECRET_KEY` is the
/// alias the connector's compose files use.
const SECRET_KEY_NAMES: [&str; 2] = ["TOON_SECRET_KEY", "NOSTR_SECRET_KEY"];
const WRITE_PORT: &str = "TOON_BLS_PORT";
const WRITE_HOST: &str = "TOON_WRITE_HOST";

const DEFAULT_WRITE_PORT: u16 = 3100;
const DEFAULT_WRITE_HOST: &str = "0.0.0.0";

/// A complete, validated configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// The node's Nostr public key, derived from its secret key.
    pub identity: PublicKey,
    /// The host `GET /health` (and later the write endpoints) listens on: an
    /// IP address or a name, resolved when the listener binds.
    pub write_host: String,
    pub write_port: u16,
}

impl Config {
    /// Read the configuration through `lookup`, which answers like
    /// `std::env::var(name).ok()`.
    ///
    /// Empty values are read as the TypeScript relay reads them. An empty port
    /// or host is the default. An empty `TOON_SECRET_KEY` is still the
    /// variable that was chosen, so it is a missing identity and does not
    /// fall through to the alias.
    pub fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, RelayError> {
        let non_empty = |name: &str| lookup(name).filter(|value| !value.is_empty());

        let (name, secret_key) = SECRET_KEY_NAMES
            .into_iter()
            .find_map(|name| lookup(name).map(|value| (name, value)))
            .filter(|(_, value)| !value.is_empty())
            .ok_or(RelayError::MissingIdentity)?;
        let identity = public_key(&secret_key).ok_or(RelayError::InvalidSecretKey { name })?;

        let write_port = match non_empty(WRITE_PORT) {
            None => DEFAULT_WRITE_PORT,
            Some(value) => match value.parse::<u16>() {
                Ok(port) if port != 0 => port,
                _ => {
                    return Err(RelayError::InvalidPort {
                        name: WRITE_PORT,
                        value,
                    });
                }
            },
        };
        let write_host = non_empty(WRITE_HOST).unwrap_or_else(|| DEFAULT_WRITE_HOST.to_string());

        Ok(Self {
            identity,
            write_host,
            write_port,
        })
    }
}

/// The x-only public key of a hex secret key, or `None` if `hex` is not one.
/// Hex only: the TypeScript relay does not accept an `nsec`.
fn public_key(hex: &str) -> Option<PublicKey> {
    if hex.len() != 64 {
        return None;
    }
    let secret_key = SecretKey::from_hex(hex).ok()?;
    Some(Keys::new(secret_key).public_key())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The x-only public keys of the secret keys `11…11` and `22…22`.
    const PUBKEY_OF_ONES: &str = "4f355bdcb7cc0af728ef3cceb9615d90684bb5b2ca5f859ab0f0b704075871aa";
    const PUBKEY_OF_TWOS: &str = "466d7fcae563e5cb09a0d1870bb580344804617879a14949cf22285f1bae3f27";

    fn config(env: &[(&str, &str)]) -> Result<Config, RelayError> {
        Config::from_env(|name| {
            env.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_string())
        })
    }

    fn ones() -> String {
        "1".repeat(64)
    }

    fn twos() -> String {
        "2".repeat(64)
    }

    #[test]
    fn a_secret_key_alone_is_a_complete_configuration_on_the_documented_port() {
        let config = config(&[("TOON_SECRET_KEY", &ones())]).expect("a secret key is enough");
        assert_eq!(config.identity.to_hex(), PUBKEY_OF_ONES);
        assert_eq!(config.write_host, "0.0.0.0");
        assert_eq!(config.write_port, 3100);
    }

    #[test]
    fn nostr_secret_key_alone_sets_the_identity() {
        let config = config(&[("NOSTR_SECRET_KEY", &twos())]).expect("the alias is enough");
        assert_eq!(config.identity.to_hex(), PUBKEY_OF_TWOS);
    }

    #[test]
    fn toon_secret_key_wins_over_its_alias() {
        let config = config(&[("NOSTR_SECRET_KEY", &twos()), ("TOON_SECRET_KEY", &ones())])
            .expect("both keys are valid");
        assert_eq!(config.identity.to_hex(), PUBKEY_OF_ONES);
    }

    #[test]
    fn an_empty_toon_secret_key_is_a_missing_identity_even_beside_the_alias() {
        let result = config(&[("TOON_SECRET_KEY", ""), ("NOSTR_SECRET_KEY", &twos())]);
        assert!(matches!(result, Err(RelayError::MissingIdentity)));
    }

    #[test]
    fn no_secret_key_is_a_missing_identity() {
        assert!(matches!(config(&[]), Err(RelayError::MissingIdentity)));
    }

    #[test]
    fn a_secret_key_that_is_not_64_hex_characters_is_refused_without_echoing_it() {
        for bad in ["not-hex", &"1".repeat(63), &"g".repeat(64), &"0".repeat(64)] {
            let error = config(&[("TOON_SECRET_KEY", bad)]).expect_err("not a secret key");
            assert!(matches!(
                error,
                RelayError::InvalidSecretKey {
                    name: "TOON_SECRET_KEY"
                }
            ));
            assert!(!error.to_string().contains(bad));
        }
    }

    #[test]
    fn the_write_listener_follows_toon_bls_port_and_toon_write_host() {
        let config = config(&[
            ("TOON_SECRET_KEY", &ones()),
            ("TOON_BLS_PORT", "3200"),
            ("TOON_WRITE_HOST", "127.0.0.1"),
        ])
        .expect("a valid port and host");
        assert_eq!(config.write_host, "127.0.0.1");
        assert_eq!(config.write_port, 3200);
    }

    #[test]
    fn a_write_host_may_be_a_name_and_an_empty_one_is_the_default() {
        let named = config(&[
            ("TOON_SECRET_KEY", &ones()),
            ("TOON_WRITE_HOST", "localhost"),
        ])
        .expect("a host name is resolved at bind, not refused here");
        assert_eq!(named.write_host, "localhost");

        let empty = config(&[
            ("TOON_SECRET_KEY", &ones()),
            ("TOON_WRITE_HOST", ""),
            ("TOON_BLS_PORT", ""),
        ])
        .expect("empty settings are the defaults");
        assert_eq!(empty.write_host, "0.0.0.0");
        assert_eq!(empty.write_port, 3100);
    }

    #[test]
    fn a_port_outside_1_to_65535_is_refused_by_name() {
        for bad in ["x", "0", "65536", "-1", "3100abc"] {
            let error = config(&[("TOON_SECRET_KEY", &ones()), ("TOON_BLS_PORT", bad)])
                .expect_err("not a port");
            assert!(matches!(
                error,
                RelayError::InvalidPort {
                    name: "TOON_BLS_PORT",
                    ..
                }
            ));
        }
    }
}
