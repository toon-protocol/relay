//! The one error type of this crate. Each variant is a way the relay refuses
//! to start, worded for the operator reading `Error: …` in a container log.

/// Why the relay did not start, or stopped.
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

    /// The write listener could not bind: the host did not resolve, or the
    /// address is taken or not ours to bind.
    #[error("could not listen on {host}:{port}: {source}")]
    Bind {
        host: String,
        port: u16,
        source: std::io::Error,
    },

    /// The listener failed after it was serving.
    #[error("the write listener stopped: {0}")]
    Serve(std::io::Error),
}
