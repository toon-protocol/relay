//! What the relay is told at startup: its flags, then its environment, then
//! the defaults, in that order of precedence. A setting that is present but
//! wrong stops the relay with a named error rather than falling back to a
//! default (#185, story 48).
//!
//! Every flag and variable of the TypeScript relay is accepted under its own
//! name with its own default, so a stack that runs the image keeps running it
//! (#200). Where the Rust relay has no surface for a setting yet it is read,
//! validated and held, not dropped: a typo in it is an error today rather than
//! on the day the surface arrives. Three settings are decided differently:
//! `TOON_DEV_MODE=true` is refused, because a relay that skips signature
//! verification is not a mode this build has; `TOON_VERIFY_WORKERS` is read
//! and has no effect, because signatures are verified natively on the
//! request's own task; and variables the relay does not know are not read.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use nostr::key::{Keys, PublicKey, SecretKey};
use nostr::nips::nip06::FromMnemonic;

use crate::{Carriage, RelayError};

/// One setting's two spellings.
struct Setting {
    flag: &'static str,
    env: &'static str,
}

const MNEMONIC: Setting = Setting {
    flag: "--mnemonic",
    env: "TOON_MNEMONIC",
};
const SECRET_KEY: Setting = Setting {
    flag: "--secret-key",
    env: "TOON_SECRET_KEY",
};
/// The alias the connector's compose files use; `TOON_SECRET_KEY` wins.
const SECRET_KEY_ALIAS: &str = "NOSTR_SECRET_KEY";
const WRITE_PORT: Setting = Setting {
    flag: "--bls-port",
    env: "TOON_BLS_PORT",
};
const WRITE_HOST: Setting = Setting {
    flag: "--write-host",
    env: "TOON_WRITE_HOST",
};
const READ_PORT: Setting = Setting {
    flag: "--relay-port",
    env: "TOON_RELAY_PORT",
};
const READ_HOST: Setting = Setting {
    flag: "--host",
    env: "TOON_HOST",
};
const DATA_DIR: Setting = Setting {
    flag: "--data-dir",
    env: "TOON_DATA_DIR",
};
const DEV_MODE: Setting = Setting {
    flag: "--dev-mode",
    env: "TOON_DEV_MODE",
};
const VERIFY_EPHEMERAL: Setting = Setting {
    flag: "--verify-ephemeral",
    env: "TOON_VERIFY_EPHEMERAL",
};
const VERIFY_WORKERS: Setting = Setting {
    flag: "--verify-workers",
    env: "TOON_VERIFY_WORKERS",
};
const MAX_CONNECTIONS: Setting = Setting {
    flag: "--max-connections",
    env: "TOON_MAX_CONNECTIONS",
};
const EPHEMERAL_RATE_LIMIT: Setting = Setting {
    flag: "--ephemeral-rate-limit",
    env: "TOON_EPHEMERAL_RATE_LIMIT",
};
const EPHEMERAL_RATE_WINDOW_MS: Setting = Setting {
    flag: "--ephemeral-rate-window-ms",
    env: "TOON_EPHEMERAL_RATE_WINDOW_MS",
};
const EPHEMERAL_MAX_BODY_BYTES: Setting = Setting {
    flag: "--ephemeral-max-body-bytes",
    env: "TOON_EPHEMERAL_MAX_BODY_BYTES",
};
const READ_RATE_LIMIT: Setting = Setting {
    flag: "--read-rate-limit",
    env: "TOON_READ_RATE_LIMIT",
};
const READ_SOURCE_RATE_LIMIT: Setting = Setting {
    flag: "--read-source-rate-limit",
    env: "TOON_READ_SOURCE_RATE_LIMIT",
};
const CONNECTOR_URL: Setting = Setting {
    flag: "--connector-url",
    env: "TOON_CONNECTOR_URL",
};
const WRITE_ILP_ADDRESS: Setting = Setting {
    flag: "--write-ilp-address",
    env: "TOON_WRITE_ILP_ADDRESS",
};
const WRITE_CARRIAGE: Setting = Setting {
    flag: "--write-carriage",
    env: "TOON_WRITE_CARRIAGE",
};
const RELAY_NAME: Setting = Setting {
    flag: "--relay-name",
    env: "TOON_RELAY_NAME",
};
const RELAY_DESCRIPTION: Setting = Setting {
    flag: "--relay-description",
    env: "TOON_RELAY_DESCRIPTION",
};
const RELAY_CONTACT: Setting = Setting {
    flag: "--relay-contact",
    env: "TOON_RELAY_CONTACT",
};
const LOG_WRITES: Setting = Setting {
    flag: "--log-writes",
    env: "TOON_LOG_WRITES",
};
const ENFORCE_EXPIRATION: Setting = Setting {
    flag: "--no-enforce-expiration",
    env: "TOON_ENFORCE_EXPIRATION",
};
const EXPIRATION_REAP_GRACE: Setting = Setting {
    flag: "--expiration-reap-grace-seconds",
    env: "TOON_EXPIRATION_REAP_GRACE_SECONDS",
};
const EXPIRATION_REAP_INTERVAL: Setting = Setting {
    flag: "--expiration-reap-interval-seconds",
    env: "TOON_EXPIRATION_REAP_INTERVAL_SECONDS",
};
const BLOCKED_EVENT_IDS: Setting = Setting {
    flag: "--blocked-event-ids",
    env: "TOON_BLOCKED_EVENT_IDS",
};

/// The flags that take a value, and the ones that stand alone.
const VALUE_FLAGS: [&str; 23] = [
    MNEMONIC.flag,
    SECRET_KEY.flag,
    READ_PORT.flag,
    WRITE_PORT.flag,
    READ_HOST.flag,
    WRITE_HOST.flag,
    DATA_DIR.flag,
    VERIFY_WORKERS.flag,
    MAX_CONNECTIONS.flag,
    EPHEMERAL_RATE_LIMIT.flag,
    EPHEMERAL_RATE_WINDOW_MS.flag,
    EPHEMERAL_MAX_BODY_BYTES.flag,
    READ_RATE_LIMIT.flag,
    READ_SOURCE_RATE_LIMIT.flag,
    CONNECTOR_URL.flag,
    WRITE_ILP_ADDRESS.flag,
    WRITE_CARRIAGE.flag,
    RELAY_NAME.flag,
    RELAY_DESCRIPTION.flag,
    RELAY_CONTACT.flag,
    EXPIRATION_REAP_GRACE.flag,
    EXPIRATION_REAP_INTERVAL.flag,
    BLOCKED_EVENT_IDS.flag,
];
const SWITCH_FLAGS: [&str; 5] = [
    DEV_MODE.flag,
    VERIFY_EPHEMERAL.flag,
    LOG_WRITES.flag,
    ENFORCE_EXPIRATION.flag,
    "--help",
];

const DEFAULT_WRITE_PORT: u16 = 3100;
const DEFAULT_READ_PORT: u16 = 7100;
const DEFAULT_HOST: &str = "0.0.0.0";
const DEFAULT_DATA_DIR: &str = "./data";
const DEFAULT_MAX_CONNECTIONS: u32 = 4096;
const DEFAULT_EPHEMERAL_RATE_LIMIT: u32 = 200;
const DEFAULT_EPHEMERAL_RATE_WINDOW_MS: u64 = 10_000;
const DEFAULT_EPHEMERAL_MAX_BODY_BYTES: u32 = 8192;
/// REQs a minute one connection is answered.
const DEFAULT_READ_RATE_LIMIT: u32 = 1_200;
/// REQs a minute all the connections of one source address are answered.
const DEFAULT_READ_SOURCE_RATE_LIMIT: u32 = 6_000;
const DEFAULT_EXPIRATION_REAP_GRACE_SECONDS: u64 = 86_400;
const DEFAULT_EXPIRATION_REAP_INTERVAL_SECONDS: u64 = 3600;
/// The most workers the TypeScript relay accepts. The setting has no effect
/// here, but a value it refused is still refused.
const MAX_VERIFY_WORKERS: u64 = 256;

/// The database file inside the data directory: the TypeScript relay's name.
const DATABASE_FILE: &str = "events.db";

/// The usage text `--help` prints.
pub const USAGE: &str = "\
Usage: relay [options]

Options (each flag beats its environment variable):
  --mnemonic <words>                       TOON_MNEMONIC
  --secret-key <hex>                       TOON_SECRET_KEY, NOSTR_SECRET_KEY
  --relay-port <port>                      TOON_RELAY_PORT (default 7100)
  --bls-port <port>                        TOON_BLS_PORT (default 3100)
  --host <host>                            TOON_HOST (default 0.0.0.0)
  --write-host <host>                      TOON_WRITE_HOST (default 0.0.0.0)
  --data-dir <path>                        TOON_DATA_DIR (default ./data)
  --dev-mode                               TOON_DEV_MODE: refused
  --verify-ephemeral                       TOON_VERIFY_EPHEMERAL
  --verify-workers <n>                     TOON_VERIFY_WORKERS: no effect
  --max-connections <n>                    TOON_MAX_CONNECTIONS (default 4096)
  --ephemeral-rate-limit <n>               TOON_EPHEMERAL_RATE_LIMIT (default 200)
  --ephemeral-rate-window-ms <n>           TOON_EPHEMERAL_RATE_WINDOW_MS (default 10000)
  --ephemeral-max-body-bytes <n>           TOON_EPHEMERAL_MAX_BODY_BYTES (default 8192)
  --read-rate-limit <n>                    TOON_READ_RATE_LIMIT (REQs a minute per connection, default 1200)
  --read-source-rate-limit <n>             TOON_READ_SOURCE_RATE_LIMIT (REQs a minute per source address, default 6000)
  --connector-url <url>                    TOON_CONNECTOR_URL
  --write-ilp-address <addr>               TOON_WRITE_ILP_ADDRESS
  --write-carriage <http|btp>              TOON_WRITE_CARRIAGE
  --relay-name <name>                      TOON_RELAY_NAME
  --relay-description <text>               TOON_RELAY_DESCRIPTION
  --relay-contact <contact>                TOON_RELAY_CONTACT
  --log-writes                             TOON_LOG_WRITES=true
  --no-enforce-expiration                  TOON_ENFORCE_EXPIRATION=false
  --expiration-reap-grace-seconds <n>      TOON_EXPIRATION_REAP_GRACE_SECONDS (default 86400)
  --expiration-reap-interval-seconds <n>   TOON_EXPIRATION_REAP_INTERVAL_SECONDS (default 3600)
  --blocked-event-ids <ids>                TOON_BLOCKED_EVENT_IDS (comma-separated)
  --help                                   show this message

Prefer the environment variables to --mnemonic and --secret-key: arguments
are visible to other users in a process listing.";

/// What the command line asked for.
#[derive(Debug)]
pub enum Invocation {
    /// Print [`USAGE`] and exit.
    Help,
    /// Start with this configuration.
    Run(Box<Config>),
}

/// The paid write edge the relay was told about: both halves, or neither.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdgeSettings {
    /// The connector's `GET /ilp` self-description URL.
    pub connector_url: String,
    /// The ILP address whose route terminates at this relay's `POST /write`.
    pub write_ilp_address: String,
}

/// A complete, validated configuration.
#[derive(Debug, Clone)]
pub struct Config {
    /// The node's Nostr public key, derived from its secret key or mnemonic.
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
    /// Verify ephemeral kinds' signatures too (`TOON_VERIFY_EPHEMERAL`).
    pub verify_ephemeral: bool,
    /// What `TOON_VERIFY_WORKERS` was set to, if it was. It has no effect: the
    /// binary says so once at startup.
    pub verify_workers: Option<u16>,
    /// The cap on concurrent read connections, 4096 unless set.
    pub max_connections: u32,
    /// The free ephemeral lane's bound: requests per key per window.
    pub ephemeral_rate_limit: u32,
    /// The free ephemeral lane's window, in milliseconds.
    pub ephemeral_rate_window_ms: u64,
    /// The free ephemeral lane's body cap, in bytes.
    pub ephemeral_max_body_bytes: u32,
    /// How many REQs a minute one read connection is answered
    /// (`TOON_READ_RATE_LIMIT`).
    pub read_rate_limit: u32,
    /// How many REQs a minute all the read connections of one source address
    /// are answered, together (`TOON_READ_SOURCE_RATE_LIMIT`).
    pub read_source_rate_limit: u32,
    /// Where the relay's writes are paid for, if it was told.
    pub edge: Option<EdgeSettings>,
    /// The carriage the paid route pins, if one was named.
    pub write_carriage: Option<Carriage>,
    /// NIP-11 `name`.
    pub relay_name: Option<String>,
    /// NIP-11 `description`.
    pub relay_description: Option<String>,
    /// NIP-11 `contact`.
    pub relay_contact: Option<String>,
    /// Log one line per accepted write.
    pub log_writes: bool,
    /// Serve no event past its NIP-40 expiration. On unless switched off.
    pub enforce_expiration: bool,
    /// How long an expired event stays on disk before it is reaped.
    pub expiration_reap_grace_seconds: u64,
    /// The reaper's sweep interval; 0 disables reaping.
    pub expiration_reap_interval_seconds: u64,
    /// Event ids the operator blocked: lower-case hex, in order, once each.
    pub blocked_event_ids: Vec<String>,
}

/// The command line split into the flags that were given.
struct Flags {
    values: HashMap<&'static str, String>,
    switches: HashSet<&'static str>,
}

impl Flags {
    /// Read `args` (without the program name). A flag is `--name value` or
    /// `--name=value`; one the relay does not have, a flag missing its value
    /// and a bare argument are errors.
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, RelayError> {
        let mut flags = Self {
            values: HashMap::new(),
            switches: HashSet::new(),
        };
        let mut args = args.into_iter();
        while let Some(arg) = args.next() {
            let Some(rest) = arg.strip_prefix("--") else {
                return Err(RelayError::UnexpectedArgument);
            };
            let (name, inline) = match rest.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (rest, None),
            };
            let name = format!("--{name}");
            if let Some(known) = VALUE_FLAGS.into_iter().find(|known| *known == name) {
                let value = match inline.or_else(|| args.next()) {
                    Some(value) => value,
                    None => return Err(RelayError::FlagNeedsValue { flag: known }),
                };
                flags.values.insert(known, value);
            } else if let Some(known) = SWITCH_FLAGS.into_iter().find(|known| *known == name) {
                if inline.is_some() {
                    return Err(RelayError::FlagTakesNoValue { flag: known });
                }
                flags.switches.insert(known);
            } else {
                return Err(RelayError::UnknownFlag { flag: name });
            }
        }
        Ok(flags)
    }
}

/// The flags and the environment, read for one setting at a time.
struct Sources<'a> {
    flags: Flags,
    lookup: &'a dyn Fn(&str) -> Option<String>,
}

impl Sources<'_> {
    /// The setting's value and the name it was given under: the flag's if the
    /// flag was given, else the variable's. An empty value is still a value.
    fn raw(&self, setting: &Setting) -> Option<(&'static str, String)> {
        match self.flags.values.get(setting.flag) {
            Some(value) => Some((setting.flag, value.clone())),
            None => (self.lookup)(setting.env).map(|value| (setting.env, value)),
        }
    }

    /// As [`Self::raw`], but empty is unset: compose's `${VAR:-}` spelling.
    fn text(&self, setting: &Setting) -> Option<(&'static str, String)> {
        self.raw(setting).filter(|(_, value)| !value.is_empty())
    }

    fn host(&self, setting: &Setting) -> String {
        self.text(setting)
            .map_or_else(|| DEFAULT_HOST.to_string(), |(_, value)| value)
    }

    fn port(&self, setting: &Setting, default: u16) -> Result<u16, RelayError> {
        match self.text(setting) {
            None => Ok(default),
            Some((name, value)) => match value.parse::<u16>() {
                Ok(port) if port != 0 => Ok(port),
                _ => Err(RelayError::InvalidPort { name, value }),
            },
        }
    }

    /// A whole number in `range`, or `None` when unset or empty.
    fn integer(
        &self,
        setting: &Setting,
        range: std::ops::RangeInclusive<u64>,
        expected: &'static str,
    ) -> Result<Option<u64>, RelayError> {
        let Some((name, value)) = self.text(setting) else {
            return Ok(None);
        };
        match value.parse::<u64>() {
            Ok(number) if range.contains(&number) => Ok(Some(number)),
            _ => Err(RelayError::InvalidSetting {
                name,
                expected,
                value,
            }),
        }
    }

    fn positive<T: TryFrom<u64>>(&self, setting: &Setting, default: T) -> Result<T, RelayError> {
        // A positive count that does not fit `T` is as invalid as a zero.
        let max = u64::from(u32::MAX);
        Ok(self
            .integer(setting, 1..=max, "a positive integer")?
            .and_then(|number| T::try_from(number).ok())
            .unwrap_or(default))
    }

    fn non_negative(&self, setting: &Setting, default: u64) -> Result<u64, RelayError> {
        Ok(self
            .integer(setting, 0..=u64::MAX, "an integer >= 0")?
            .unwrap_or(default))
    }

    /// A switch: its flag, or its variable set to exactly `true`.
    fn on(&self, setting: &Setting) -> bool {
        self.flags.switches.contains(setting.flag)
            || (self.lookup)(setting.env).is_some_and(|value| value == "true")
    }
}

impl Config {
    /// Read the configuration from the environment alone, through `lookup`,
    /// which answers like `std::env::var(name).ok()`.
    pub fn from_env(lookup: impl Fn(&str) -> Option<String>) -> Result<Self, RelayError> {
        match Self::from_args_and_env(std::iter::empty(), lookup)? {
            Invocation::Run(config) => Ok(*config),
            Invocation::Help => unreachable!("no flag was given, so help was not asked for"),
        }
    }

    /// Read the configuration from `args` (without the program name) and, for
    /// what they leave unset, the environment through `lookup`.
    ///
    /// Empty values are read as the TypeScript relay reads them. An empty port,
    /// host, data directory or number is the default. An empty
    /// `TOON_SECRET_KEY` is still the variable that was chosen, so it is a
    /// missing identity and does not fall through to the alias.
    pub fn from_args_and_env(
        args: impl IntoIterator<Item = String>,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Invocation, RelayError> {
        let flags = Flags::parse(args)?;
        if flags.switches.contains("--help") {
            return Ok(Invocation::Help);
        }
        let sources = Sources {
            flags,
            lookup: &lookup,
        };

        if sources.on(&DEV_MODE) {
            return Err(RelayError::DevModeRefused);
        }

        let identity = sources.identity()?;
        let verify_workers = sources
            .integer(
                &VERIFY_WORKERS,
                0..=MAX_VERIFY_WORKERS,
                "an integer between 0 and 256",
            )?
            .and_then(|workers| u16::try_from(workers).ok());

        let edge = match (
            sources.text(&CONNECTOR_URL),
            sources.text(&WRITE_ILP_ADDRESS),
        ) {
            (Some((_, connector_url)), Some((_, write_ilp_address))) => {
                // Checked here, not at the first poll: a URL the relay can
                // never ask would otherwise start a relay that advertises
                // nothing, one log line, forever. The connector is on the
                // relay's own network, so it is plain HTTP.
                let asked = connector_url
                    .parse::<hyper::Uri>()
                    .is_ok_and(|uri| uri.scheme_str() == Some("http") && uri.authority().is_some());
                if !asked {
                    return Err(RelayError::InvalidConnectorUrl {
                        value: connector_url,
                    });
                }
                Some(EdgeSettings {
                    connector_url,
                    write_ilp_address,
                })
            }
            (None, None) => None,
            (Some(_), None) => {
                return Err(RelayError::EdgeIncomplete {
                    given: CONNECTOR_URL.env,
                    missing: WRITE_ILP_ADDRESS.env,
                });
            }
            (None, Some(_)) => {
                return Err(RelayError::EdgeIncomplete {
                    given: WRITE_ILP_ADDRESS.env,
                    missing: CONNECTOR_URL.env,
                });
            }
        };
        let write_carriage = match sources.text(&WRITE_CARRIAGE) {
            None => None,
            Some((_, value)) if value == "http" => Some(Carriage::Http),
            Some((_, value)) if value == "btp" => Some(Carriage::Btp),
            Some((name, value)) => return Err(RelayError::InvalidCarriage { name, value }),
        };

        let (blocked_event_ids, rejected) = blocked_ids(
            &sources
                .raw(&BLOCKED_EVENT_IDS)
                .map(|(_, value)| value)
                .unwrap_or_default(),
        );
        if !rejected.is_empty() {
            return Err(RelayError::InvalidBlockedEventIds { rejected });
        }

        let text = |setting: &Setting| sources.text(setting).map(|(_, value)| value);
        let enforce_expiration = !(sources.flags.switches.contains(ENFORCE_EXPIRATION.flag)
            || lookup(ENFORCE_EXPIRATION.env).is_some_and(|value| value == "false"));

        Ok(Invocation::Run(Box::new(Self {
            identity,
            write_host: sources.host(&WRITE_HOST),
            write_port: sources.port(&WRITE_PORT, DEFAULT_WRITE_PORT)?,
            read_host: sources.host(&READ_HOST),
            read_port: sources.port(&READ_PORT, DEFAULT_READ_PORT)?,
            data_dir: text(&DATA_DIR)
                .unwrap_or_else(|| DEFAULT_DATA_DIR.to_string())
                .into(),
            verify_ephemeral: sources.on(&VERIFY_EPHEMERAL),
            verify_workers,
            max_connections: sources.positive(&MAX_CONNECTIONS, DEFAULT_MAX_CONNECTIONS)?,
            ephemeral_rate_limit: sources
                .positive(&EPHEMERAL_RATE_LIMIT, DEFAULT_EPHEMERAL_RATE_LIMIT)?,
            ephemeral_rate_window_ms: sources
                .positive(&EPHEMERAL_RATE_WINDOW_MS, DEFAULT_EPHEMERAL_RATE_WINDOW_MS)?,
            ephemeral_max_body_bytes: sources
                .positive(&EPHEMERAL_MAX_BODY_BYTES, DEFAULT_EPHEMERAL_MAX_BODY_BYTES)?,
            read_rate_limit: sources.positive(&READ_RATE_LIMIT, DEFAULT_READ_RATE_LIMIT)?,
            read_source_rate_limit: sources
                .positive(&READ_SOURCE_RATE_LIMIT, DEFAULT_READ_SOURCE_RATE_LIMIT)?,
            edge,
            write_carriage,
            relay_name: text(&RELAY_NAME),
            relay_description: text(&RELAY_DESCRIPTION),
            relay_contact: text(&RELAY_CONTACT),
            log_writes: sources.on(&LOG_WRITES),
            enforce_expiration,
            expiration_reap_grace_seconds: sources.non_negative(
                &EXPIRATION_REAP_GRACE,
                DEFAULT_EXPIRATION_REAP_GRACE_SECONDS,
            )?,
            expiration_reap_interval_seconds: sources.non_negative(
                &EXPIRATION_REAP_INTERVAL,
                DEFAULT_EXPIRATION_REAP_INTERVAL_SECONDS,
            )?,
            blocked_event_ids,
        })))
    }

    /// Where the database is: `events.db` in the data directory.
    pub fn database_path(&self) -> PathBuf {
        self.data_dir.join(DATABASE_FILE)
    }
}

impl Sources<'_> {
    /// The node's identity: a mnemonic or a secret key, never both.
    ///
    /// The secret key is the flag, else `TOON_SECRET_KEY`, else its alias; an
    /// empty one that was chosen is no key at all.
    fn identity(&self) -> Result<PublicKey, RelayError> {
        let secret_key = self
            .raw(&SECRET_KEY)
            .or_else(|| (self.lookup)(SECRET_KEY_ALIAS).map(|value| (SECRET_KEY_ALIAS, value)));
        let secret_key = secret_key.filter(|(_, value)| !value.is_empty());
        let mnemonic = self.text(&MNEMONIC);

        match (mnemonic, secret_key) {
            (Some(_), Some(_)) => Err(RelayError::BothIdentities),
            (None, None) => Err(RelayError::MissingIdentity),
            (None, Some((name, hex))) => {
                public_key(&hex).ok_or(RelayError::InvalidSecretKey { name })
            }
            (Some((name, words)), None) => Keys::from_mnemonic(words, None)
                .map(|keys| keys.public_key())
                .map_err(|_| RelayError::InvalidMnemonic { name }),
        }
    }
}

/// The ids in `raw` (separated by commas or white space) as lower-case hex,
/// each once, and the entries that are not 64 hex characters.
fn blocked_ids(raw: &str) -> (Vec<String>, Vec<String>) {
    let mut ids: Vec<String> = Vec::new();
    let mut rejected = Vec::new();
    for entry in raw
        .split(|c: char| c == ',' || c.is_whitespace())
        .filter(|entry| !entry.is_empty())
    {
        let id = entry.to_ascii_lowercase();
        if id.len() == 64 && id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            if !ids.contains(&id) {
                ids.push(id);
            }
        } else {
            rejected.push(entry.to_string());
        }
    }
    (ids, rejected)
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

    fn run(args: &[&str], env: &[(&str, &str)]) -> Result<Config, RelayError> {
        let args = args.iter().map(|arg| (*arg).to_string());
        let lookup = |name: &str| {
            env.iter()
                .find(|(key, _)| *key == name)
                .map(|(_, value)| (*value).to_string())
        };
        match Config::from_args_and_env(args, lookup)? {
            Invocation::Run(config) => Ok(*config),
            Invocation::Help => panic!("help was not asked for"),
        }
    }

    #[test]
    fn every_setting_has_the_typescript_relays_default() {
        let config = config(&[("TOON_SECRET_KEY", &ones())]).expect("a key is enough");
        assert_eq!(config.max_connections, 4096);
        assert_eq!(config.ephemeral_rate_limit, 200);
        assert_eq!(config.ephemeral_rate_window_ms, 10_000);
        assert_eq!(config.ephemeral_max_body_bytes, 8192);
        assert_eq!(config.read_rate_limit, 1200);
        assert_eq!(config.read_source_rate_limit, 6000);
        assert_eq!(config.expiration_reap_grace_seconds, 86_400);
        assert_eq!(config.expiration_reap_interval_seconds, 3600);
        assert!(config.enforce_expiration);
        assert!(!config.verify_ephemeral && !config.log_writes);
        assert_eq!(config.verify_workers, None);
        assert!(config.edge.is_none() && config.write_carriage.is_none());
        assert!(config.blocked_event_ids.is_empty());
    }

    #[test]
    fn a_flag_beats_its_variable() {
        let config = run(
            &[
                "--bls-port",
                "3300",
                "--relay-name=flagged",
                "--max-connections",
                "9",
            ],
            &[
                ("TOON_SECRET_KEY", &ones()),
                ("TOON_BLS_PORT", "3200"),
                ("TOON_RELAY_NAME", "from env"),
                ("TOON_MAX_CONNECTIONS", "5"),
            ],
        )
        .expect("both are valid");
        assert_eq!(config.write_port, 3300);
        assert_eq!(config.relay_name.as_deref(), Some("flagged"));
        assert_eq!(config.max_connections, 9);
    }

    #[test]
    fn a_secret_key_flag_beats_both_variables() {
        let config =
            run(&["--secret-key", &twos()], &[("TOON_SECRET_KEY", &ones())]).expect("valid");
        assert_eq!(config.identity.to_hex(), PUBKEY_OF_TWOS);
    }

    #[test]
    fn a_flag_that_is_invalid_is_refused_even_when_the_variable_is_valid() {
        let error = run(
            &["--relay-port", "x"],
            &[("TOON_SECRET_KEY", &ones()), ("TOON_RELAY_PORT", "7200")],
        )
        .expect_err("the flag wins, and is wrong");
        assert!(matches!(
            error,
            RelayError::InvalidPort {
                name: "--relay-port",
                ..
            }
        ));
    }

    #[test]
    fn unknown_flags_and_stray_arguments_are_errors_and_unknown_variables_are_not() {
        let env = [("TOON_SECRET_KEY", ones()), ("TOON_NO_SUCH", "1".into())];
        let env: Vec<(&str, &str)> = env.iter().map(|(k, v)| (*k, v.as_str())).collect();
        assert!(run(&[], &env).is_ok());
        assert!(matches!(
            run(&["--nope"], &env),
            Err(RelayError::UnknownFlag { .. })
        ));
        assert!(matches!(
            run(&["stray"], &env),
            Err(RelayError::UnexpectedArgument)
        ));
        // An unquoted mnemonic spills its words into bare arguments; none is echoed.
        let spilled = run(&["--mnemonic", "abandon", "ability"], &env)
            .err()
            .map(|error| error.to_string());
        assert!(
            spilled.as_deref().is_some_and(|m| !m.contains("ability")),
            "{spilled:?}"
        );
        assert!(matches!(
            run(&["--host"], &env),
            Err(RelayError::FlagNeedsValue { .. })
        ));
        assert!(matches!(
            run(&["--log-writes=yes"], &env),
            Err(RelayError::FlagTakesNoValue { .. })
        ));
    }

    #[test]
    fn help_is_a_request_to_print_usage_not_to_start() {
        let invocation = Config::from_args_and_env(["--help".to_string()], |_| None)
            .expect("help needs no identity");
        assert!(matches!(invocation, Invocation::Help));
    }

    #[test]
    fn dev_mode_true_is_refused_and_false_or_unset_is_not() {
        let key = ones();
        assert!(matches!(
            config(&[("TOON_SECRET_KEY", &key), ("TOON_DEV_MODE", "true")]),
            Err(RelayError::DevModeRefused)
        ));
        assert!(matches!(
            run(&["--dev-mode"], &[("TOON_SECRET_KEY", &key)]),
            Err(RelayError::DevModeRefused)
        ));
        for value in ["false", "", "1", "TRUE"] {
            assert!(config(&[("TOON_SECRET_KEY", &key), ("TOON_DEV_MODE", value)]).is_ok());
        }
    }

    #[test]
    fn verify_workers_is_read_and_a_value_the_typescript_relay_refuses_is_refused() {
        let key = ones();
        let ok = config(&[("TOON_SECRET_KEY", &key), ("TOON_VERIFY_WORKERS", "0")])
            .expect("0 is accepted");
        assert_eq!(ok.verify_workers, Some(0));
        for bad in ["-1", "257", "x"] {
            let result = config(&[("TOON_SECRET_KEY", &key), ("TOON_VERIFY_WORKERS", bad)]);
            assert!(
                matches!(result, Err(RelayError::InvalidSetting { .. })),
                "{bad}"
            );
        }
    }

    #[test]
    fn counts_must_be_positive_and_retention_may_be_zero() {
        let key = ones();
        for name in [
            "TOON_MAX_CONNECTIONS",
            "TOON_EPHEMERAL_RATE_LIMIT",
            "TOON_EPHEMERAL_RATE_WINDOW_MS",
            "TOON_EPHEMERAL_MAX_BODY_BYTES",
            "TOON_READ_RATE_LIMIT",
            "TOON_READ_SOURCE_RATE_LIMIT",
        ] {
            for bad in ["0", "-1", "x"] {
                let result = config(&[("TOON_SECRET_KEY", &key), (name, bad)]);
                assert!(
                    matches!(result, Err(RelayError::InvalidSetting { name: refused, .. }) if refused == name),
                    "{name}={bad}"
                );
            }
            assert!(config(&[("TOON_SECRET_KEY", &key), (name, "")]).is_ok());
        }
        let config = config(&[
            ("TOON_SECRET_KEY", &key),
            ("TOON_EXPIRATION_REAP_GRACE_SECONDS", "0"),
            ("TOON_EXPIRATION_REAP_INTERVAL_SECONDS", "0"),
        ])
        .expect("0 is meaningful for retention");
        assert_eq!(config.expiration_reap_grace_seconds, 0);
        assert_eq!(config.expiration_reap_interval_seconds, 0);
    }

    #[test]
    fn only_the_exact_strings_the_typescript_relay_reads_switch_a_boolean() {
        let key = ones();
        let on = config(&[
            ("TOON_SECRET_KEY", &key),
            ("TOON_LOG_WRITES", "true"),
            ("TOON_VERIFY_EPHEMERAL", "true"),
            ("TOON_ENFORCE_EXPIRATION", "false"),
        ])
        .expect("valid");
        assert!(on.log_writes && on.verify_ephemeral && !on.enforce_expiration);
        // A typo fails towards enforcing expiration.
        let typo = config(&[
            ("TOON_SECRET_KEY", &key),
            ("TOON_ENFORCE_EXPIRATION", "False"),
        ])
        .expect("valid");
        assert!(typo.enforce_expiration);
        assert!(
            run(&["--log-writes"], &[("TOON_SECRET_KEY", &key)])
                .expect("valid")
                .log_writes
        );
        assert!(
            !run(&["--no-enforce-expiration"], &[("TOON_SECRET_KEY", &key)])
                .expect("valid")
                .enforce_expiration
        );
    }

    #[test]
    fn the_connector_url_and_the_write_address_go_together() {
        let key = ones();
        let both = config(&[
            ("TOON_SECRET_KEY", &key),
            ("TOON_CONNECTOR_URL", "http://connector:3000/ilp"),
            ("TOON_WRITE_ILP_ADDRESS", "g.toon.relay"),
        ])
        .expect("both set");
        assert_eq!(
            both.edge,
            Some(EdgeSettings {
                connector_url: "http://connector:3000/ilp".into(),
                write_ilp_address: "g.toon.relay".into()
            })
        );
        for one in ["TOON_CONNECTOR_URL", "TOON_WRITE_ILP_ADDRESS"] {
            let result = config(&[("TOON_SECRET_KEY", &key), (one, "x")]);
            assert!(
                matches!(result, Err(RelayError::EdgeIncomplete { given, .. }) if given == one)
            );
        }
        // Empty is compose's way of saying unset.
        let empty = config(&[
            ("TOON_SECRET_KEY", &key),
            ("TOON_CONNECTOR_URL", ""),
            ("TOON_WRITE_ILP_ADDRESS", ""),
        ])
        .expect("both unset");
        assert!(empty.edge.is_none());
    }

    #[test]
    fn a_connector_url_the_relay_cannot_ask_refuses_the_start() {
        for url in [
            "connector:3000/ilp",
            "https://connector:3000/ilp",
            "http://",
            "not a url",
        ] {
            let result = config(&[
                ("TOON_SECRET_KEY", &ones()),
                ("TOON_CONNECTOR_URL", url),
                ("TOON_WRITE_ILP_ADDRESS", "g.toon.relay"),
            ]);
            assert!(
                matches!(result, Err(RelayError::InvalidConnectorUrl { .. })),
                "{url}"
            );
        }
    }

    #[test]
    fn the_carriage_is_http_or_btp_and_nothing_else() {
        let key = ones();
        let btp = config(&[("TOON_SECRET_KEY", &key), ("TOON_WRITE_CARRIAGE", "btp")])
            .expect("btp is a carriage");
        assert_eq!(btp.write_carriage, Some(Carriage::Btp));
        let none = config(&[("TOON_SECRET_KEY", &key), ("TOON_WRITE_CARRIAGE", "")])
            .expect("empty is unset");
        assert_eq!(none.write_carriage, None);
        let both = config(&[("TOON_SECRET_KEY", &key), ("TOON_WRITE_CARRIAGE", "both")]);
        assert!(matches!(both, Err(RelayError::InvalidCarriage { .. })));
    }

    #[test]
    fn blocked_ids_are_lower_cased_deduplicated_and_a_malformed_one_is_refused() {
        let key = ones();
        let upper = "AA".repeat(32);
        let list = format!("{upper}, {} {}", "aa".repeat(32), "bb".repeat(32));
        let config = config(&[("TOON_SECRET_KEY", &key), ("TOON_BLOCKED_EVENT_IDS", &list)])
            .expect("valid ids");
        assert_eq!(config.blocked_event_ids, ["aa".repeat(32), "bb".repeat(32)]);

        let bad = format!("{},zz,12", "aa".repeat(32));
        let error = config_error(&key, &bad);
        assert!(matches!(
            error,
            RelayError::InvalidBlockedEventIds { rejected } if rejected == ["zz", "12"]
        ));
    }

    fn config_error(key: &str, blocked: &str) -> RelayError {
        config(&[
            ("TOON_SECRET_KEY", key),
            ("TOON_BLOCKED_EVENT_IDS", blocked),
        ])
        .expect_err("a malformed id")
    }

    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    #[test]
    fn a_mnemonic_alone_sets_the_identity_and_is_never_echoed_when_wrong() {
        let config = config(&[("TOON_MNEMONIC", ABANDON)]).expect("a valid mnemonic");
        assert_eq!(config.identity.to_hex().len(), 64);
        let error = run(&["--mnemonic", "not a mnemonic"], &[]).expect_err("invalid words");
        assert!(matches!(
            error,
            RelayError::InvalidMnemonic { name: "--mnemonic" }
        ));
        assert!(!error.to_string().contains("not a mnemonic"));
    }

    #[test]
    fn a_mnemonic_and_a_secret_key_together_are_refused() {
        let result = config(&[("TOON_MNEMONIC", ABANDON), ("TOON_SECRET_KEY", &ones())]);
        assert!(matches!(result, Err(RelayError::BothIdentities)));
    }
}
