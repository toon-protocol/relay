//! What the relay is told at startup. A setting that is present but wrong
//! stops the relay with a named error rather than falling back to a default
//! (#185, story 48).
//!
//! The connector settings are the TypeScript relay's: `TOON_CONNECTOR_URL`
//! and `TOON_WRITE_ILP_ADDRESS` go together or not at all, and
//! `TOON_WRITE_CARRIAGE` only fills the connector's silence (#199).
//!
//! Only the settings a built surface acts on are read. The rest of the
//! TypeScript relay's environment and its flags arrive with the surfaces they
//! configure (#200).

use std::path::PathBuf;

use nostr::key::{Keys, PublicKey, SecretKey};

use crate::{Carriage, RelayError};

/// The identity variables, in precedence order: `NOSTR_SECRET_KEY` is the
/// alias the connector's compose files use.
const SECRET_KEY_NAMES: [&str; 2] = ["TOON_SECRET_KEY", "NOSTR_SECRET_KEY"];
const WRITE_PORT: &str = "TOON_BLS_PORT";
const WRITE_HOST: &str = "TOON_WRITE_HOST";
const READ_PORT: &str = "TOON_RELAY_PORT";
const READ_HOST: &str = "TOON_HOST";
const DATA_DIR: &str = "TOON_DATA_DIR";
const CONNECTOR_URL: &str = "TOON_CONNECTOR_URL";
const WRITE_ILP_ADDRESS: &str = "TOON_WRITE_ILP_ADDRESS";
const WRITE_CARRIAGE: &str = "TOON_WRITE_CARRIAGE";
const ENFORCE_EXPIRATION: &str = "TOON_ENFORCE_EXPIRATION";
const RELAY_NAME: &str = "TOON_RELAY_NAME";
const RELAY_DESCRIPTION: &str = "TOON_RELAY_DESCRIPTION";
const RELAY_CONTACT: &str = "TOON_RELAY_CONTACT";

const DEFAULT_WRITE_PORT: u16 = 3100;
const DEFAULT_READ_PORT: u16 = 7100;
const DEFAULT_HOST: &str = "0.0.0.0";
const DEFAULT_DATA_DIR: &str = "./data";

/// The database file inside the data directory: the TypeScript relay's name.
const DATABASE_FILE: &str = "events.db";

/// A complete, validated configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// The node's Nostr public key, derived from its secret key.
    pub identity: PublicKey,
    /// The host the write port (`POST /write`, `GET /health`) listens on: an
    /// IP address or a name, resolved when the listener binds.
    pub write_host: String,
    /// The write port: `TOON_BLS_PORT`, 3100 unless set.
    pub write_port: u16,
    /// The host the read side (the NIP-01 WebSocket) listens on.
    pub read_host: String,
    /// The read port: `TOON_RELAY_PORT`, 7100 unless set.
    pub read_port: u16,
    /// The directory that holds the database, created if it is missing.
    pub data_dir: PathBuf,
    /// The connector to read the Write Edge from, or `None` for a relay that
    /// publishes no edge.
    pub connector: Option<ConnectorConfig>,
    /// `TOON_WRITE_CARRIAGE`: the carriage the document states where the
    /// connector states none.
    pub write_carriage: Option<Carriage>,
    /// Whether NIP-40 expiration is claimed: `TOON_ENFORCE_EXPIRATION`, on
    /// unless it is exactly `false`.
    pub enforce_expiration: bool,
    /// The operator's free text for the information document.
    pub description: Description,
}

/// The two settings that say where the Write Edge is read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectorConfig {
    /// The connector's self-description URL, `http://connector:3000/ilp`.
    pub url: String,
    /// The address whose route terminates at this relay's `POST /write`: the
    /// one thing the relay is told rather than reads.
    pub ilp_address: String,
}

/// The operator's `name`, `description` and `contact` for the information
/// document. An empty setting is an absent one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Description {
    pub name: Option<String>,
    pub description: Option<String>,
    pub contact: Option<String>,
}

impl Config {
    /// Read the configuration through `lookup`, which answers like
    /// `std::env::var(name).ok()`.
    ///
    /// Empty values are read as the TypeScript relay reads them. An empty port,
    /// host or data directory is the default. An empty `TOON_SECRET_KEY` is still the
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

        let port = |name: &'static str, default: u16| match non_empty(name) {
            None => Ok(default),
            Some(value) => match value.parse::<u16>() {
                Ok(port) if port != 0 => Ok(port),
                _ => Err(RelayError::InvalidPort { name, value }),
            },
        };
        let host = |name: &str| non_empty(name).unwrap_or_else(|| DEFAULT_HOST.to_string());

        // Both or neither: an address nobody checked against a connector is
        // an address clients' money could be sent to and refused.
        let connector = match (non_empty(CONNECTOR_URL), non_empty(WRITE_ILP_ADDRESS)) {
            (Some(url), Some(ilp_address)) => Some(ConnectorConfig { url, ilp_address }),
            (None, None) => None,
            _ => return Err(RelayError::ConnectorSettingsApart),
        };
        let write_carriage = match non_empty(WRITE_CARRIAGE).as_deref() {
            None => None,
            Some("http") => Some(Carriage::Http),
            Some("btp") => Some(Carriage::Btp),
            Some(value) => {
                return Err(RelayError::InvalidCarriage {
                    value: value.to_string(),
                });
            }
        };

        Ok(Self {
            identity,
            write_host: host(WRITE_HOST),
            write_port: port(WRITE_PORT, DEFAULT_WRITE_PORT)?,
            read_host: host(READ_HOST),
            read_port: port(READ_PORT, DEFAULT_READ_PORT)?,
            data_dir: non_empty(DATA_DIR)
                .unwrap_or_else(|| DEFAULT_DATA_DIR.to_string())
                .into(),
            connector,
            write_carriage,
            enforce_expiration: lookup(ENFORCE_EXPIRATION).as_deref() != Some("false"),
            description: Description {
                name: non_empty(RELAY_NAME),
                description: non_empty(RELAY_DESCRIPTION),
                contact: non_empty(RELAY_CONTACT),
            },
        })
    }

    /// Where the database is: `events.db` in the data directory.
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join(DATABASE_FILE)
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
    use std::path::Path;

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
        for name in ["TOON_BLS_PORT", "TOON_RELAY_PORT"] {
            for bad in ["x", "0", "65536", "-1", "3100abc"] {
                let error =
                    config(&[("TOON_SECRET_KEY", &ones()), (name, bad)]).expect_err("not a port");
                assert!(
                    matches!(error, RelayError::InvalidPort { name: refused, .. } if refused == name)
                );
            }
        }
    }

    #[test]
    fn the_read_side_defaults_to_port_7100_on_every_interface() {
        let config = config(&[("TOON_SECRET_KEY", &ones())]).expect("a secret key is enough");
        assert_eq!(config.read_host, "0.0.0.0");
        assert_eq!(config.read_port, 7100);
    }

    #[test]
    fn the_read_listener_follows_toon_relay_port_and_toon_host() {
        let config = config(&[
            ("TOON_SECRET_KEY", &ones()),
            ("TOON_RELAY_PORT", "7200"),
            ("TOON_HOST", "127.0.0.1"),
        ])
        .expect("a valid port and host");
        assert_eq!(config.read_host, "127.0.0.1");
        assert_eq!(config.read_port, 7200);
    }

    #[test]
    fn the_database_is_events_db_in_the_data_directory() {
        let default = config(&[("TOON_SECRET_KEY", &ones())]).expect("a secret key is enough");
        assert_eq!(default.database_path(), Path::new("./data/events.db"));

        let moved = config(&[("TOON_SECRET_KEY", &ones()), ("TOON_DATA_DIR", "/data")])
            .expect("a data directory");
        assert_eq!(moved.database_path(), Path::new("/data/events.db"));

        let empty = config(&[("TOON_SECRET_KEY", &ones()), ("TOON_DATA_DIR", "")])
            .expect("an empty data directory is the default");
        assert_eq!(empty.database_path(), Path::new("./data/events.db"));
    }

    #[test]
    fn the_connector_url_and_the_write_address_go_together() {
        let both = config(&[
            ("TOON_SECRET_KEY", &ones()),
            ("TOON_CONNECTOR_URL", "http://connector:3000/ilp"),
            ("TOON_WRITE_ILP_ADDRESS", "g.toon.relay"),
        ])
        .expect("both settings are a connector");
        assert_eq!(
            both.connector,
            Some(ConnectorConfig {
                url: "http://connector:3000/ilp".to_string(),
                ilp_address: "g.toon.relay".to_string(),
            })
        );

        let neither = config(&[
            ("TOON_SECRET_KEY", &ones()),
            ("TOON_CONNECTOR_URL", ""),
            ("TOON_WRITE_ILP_ADDRESS", ""),
        ])
        .expect("empty settings are unset");
        assert_eq!(neither.connector, None);

        for lone in ["TOON_CONNECTOR_URL", "TOON_WRITE_ILP_ADDRESS"] {
            let result = config(&[("TOON_SECRET_KEY", &ones()), (lone, "x")]);
            assert!(matches!(result, Err(RelayError::ConnectorSettingsApart)));
        }
    }

    #[test]
    fn a_write_carriage_is_http_btp_or_unset() {
        let carriage = |value: &str| {
            config(&[("TOON_SECRET_KEY", &ones()), ("TOON_WRITE_CARRIAGE", value)])
                .map(|config| config.write_carriage)
        };
        assert_eq!(carriage("http").expect("http"), Some(Carriage::Http));
        assert_eq!(carriage("btp").expect("btp"), Some(Carriage::Btp));
        assert_eq!(carriage("").expect("empty is unset"), None);
        assert!(matches!(
            carriage("both"),
            Err(RelayError::InvalidCarriage { .. })
        ));
    }

    #[test]
    fn expiration_is_claimed_unless_switched_off_with_exactly_false() {
        let enforced = |value: Option<&str>| {
            let mut env = vec![("TOON_SECRET_KEY", ones())];
            if let Some(value) = value {
                env.push(("TOON_ENFORCE_EXPIRATION", value.to_string()));
            }
            let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
            config(&env).expect("valid").enforce_expiration
        };
        assert!(enforced(None));
        assert!(enforced(Some("true")));
        assert!(enforced(Some("FALSE")));
        assert!(!enforced(Some("false")));
    }

    #[test]
    fn the_operators_free_text_is_kept_unless_empty() {
        let config = config(&[
            ("TOON_SECRET_KEY", &ones()),
            ("TOON_RELAY_NAME", "devnet"),
            ("TOON_RELAY_DESCRIPTION", ""),
        ])
        .expect("valid");
        assert_eq!(config.description.name.as_deref(), Some("devnet"));
        assert_eq!(config.description.description, None);
        assert_eq!(config.description.contact, None);
    }
}
