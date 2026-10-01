//! The one error type of this crate. A variant is either a way the relay
//! refuses to start, worded for the operator reading `Error: …` in a container
//! log, or a way it refuses one request, which the handler turns into a status.

/// Why the relay did not start, stopped, or refused a request.
#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    /// No identity variable is set, or the one that was chosen is empty.
    #[error(
        "a secret key is required: set TOON_SECRET_KEY (or NOSTR_SECRET_KEY) \
         to a 64-character hex string"
    )]
    MissingIdentity,

    /// `name` holds something that is not a secret key. The value is
    /// deliberately not echoed: it is, or was meant to be, a secret.
    #[error("{name} must be a 64-character hex string that is a valid secp256k1 secret key")]
    InvalidSecretKey { name: &'static str },

    /// `name` is set to something that is not a whole port number.
    #[error("{name} must be an integer between 1 and 65535, got {value:?}")]
    InvalidPort { name: &'static str, value: String },

    /// A listener could not bind: the host did not resolve, or the address is
    /// taken or not ours to bind.
    #[error("could not listen on {host}:{port}: {source}")]
    Bind {
        host: String,
        port: u16,
        source: std::io::Error,
    },

    /// A listener failed after it was serving.
    #[error("a listener stopped: {0}")]
    Serve(std::io::Error),

    /// The data directory is not there and could not be created.
    #[error("could not create the data directory {}: {source}", path.display())]
    DataDir {
        path: std::path::PathBuf,
        source: std::io::Error,
    },

    /// A read-side connection ended on an error the framework reported: the
    /// peer broke the protocol, or went over a limit.
    #[error("{0}")]
    ReadSide(String),

    /// The event's id is not the hash of its content: it was altered after
    /// it was signed, or never had an honest id.
    #[error("the event id does not match the event")]
    EventIdMismatch,

    /// The event's signature is not its author's signature over its id.
    #[error("the event signature is not valid for its author and id")]
    EventSignatureInvalid,

    /// The database file could not be opened, or holds a schema that is not
    /// the relay's.
    #[error("could not open the event store at {}: {source}", path.display())]
    StoreOpen {
        path: std::path::PathBuf,
        source: rusqlite::Error,
    },

    /// SQLite refused a read or a write.
    #[error("the event store failed: {0}")]
    Store(#[from] rusqlite::Error),

    /// The task running a store call ended without an answer: it panicked,
    /// or the runtime is shutting down.
    #[error("the event store did not answer")]
    StoreStopped,

    /// The event's tags could not be written as JSON.
    #[error("the event's tags could not be encoded: {0}")]
    TagsNotJson(serde_json::Error),

    /// The event is of a kind whose storage rule this build does not have
    /// yet: replaceable, addressable or ephemeral (#195, #198). It was not stored.
    #[error("events of kind {kind} are not stored by this build yet")]
    KindNotStoredYet { kind: u16 },

    /// A retention setting that is not a whole number of seconds.
    #[error("{name} must be an integer >= 0, got {value:?}")]
    InvalidSeconds { name: &'static str, value: String },

    /// `TOON_BLOCKED_EVENT_IDS` names something that is not an event id. A
    /// blocklist that quietly skipped an entry would leave an operator
    /// believing an event is blocked while the relay keeps serving it.
    #[error("TOON_BLOCKED_EVENT_IDS entries must be 64-character hex event ids; rejected: {}", rejected.join(", "))]
    InvalidBlockedEventIds { rejected: Vec<String> },

    /// The address this relay was told its writes are paid at is not the
    /// prefix of any route its connector publishes. Nothing is advertised.
    #[error(
        "the connector does not terminate `{address}` — it terminates {}. Point \
         TOON_WRITE_ILP_ADDRESS at the prefix whose route reaches this relay's POST /write",
        listed(terminated)
    )]
    RouteNotTerminated {
        address: String,
        /// Every prefix the connector does terminate, in its order.
        terminated: Vec<String>,
    },

    /// The connector's self-description has no `httpEndpoint`.
    #[error(
        "the connector publishes no `httpEndpoint`, so it has no URL to send clients to \
         (its [node] section is unset)"
    )]
    ConnectorPublishesNoUrl,

    /// The connector's self-description has no `edgeIdentity.publicKey`.
    #[error(
        "the connector publishes no `edgeIdentity.publicKey`, so there is no key for a \
         client to seal a write to"
    )]
    ConnectorPublishesNoSealKey,

    /// The connector prices the relay's route at something that is not a
    /// whole number of base units a JSON number carries exactly.
    #[error("the connector prices `{address}` at {price:?}, which is not a whole number of uusdc")]
    RoutePriceNotWhole { address: String, price: String },
}

/// `a, b, c`, or `nothing` for an empty list.
fn listed(prefixes: &[String]) -> String {
    if prefixes.is_empty() {
        "nothing".to_string()
    } else {
        prefixes.join(", ")
    }
}
