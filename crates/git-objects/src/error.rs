//! The one error enum: every way an object can fail to be usable, as values
//! the UI can show rather than panics.

use crate::{ObjectId, ObjectKind};

/// Why an object could not be used. Each variant is a distinct cause, which
/// the UI shows as "no longer held".
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ObjectError {
    /// The store has no object for this id (or the fetch callback said so).
    #[error("object {0} is not held")]
    Missing(ObjectId),
    /// The bytes do not hash to the id they were asked for.
    #[error("object {expected} is not what the store returned: the bytes hash to {actual}")]
    Mismatch {
        expected: ObjectId,
        actual: ObjectId,
    },
    /// The object is larger than the caller allows.
    #[error("object is {size} bytes, over the limit of {limit}")]
    Oversized { size: u64, limit: u64 },
    /// The object is not zlib data, or its header is unreadable.
    #[error("loose object is malformed: {0}")]
    BadLoose(&'static str),
    /// The object parsed as the wrong kind for what the caller needs.
    #[error("expected a {expected} object, found a {found}")]
    WrongKind {
        expected: ObjectKind,
        found: ObjectKind,
    },
    /// The body does not follow its kind's format.
    #[error("{kind} object is malformed: {reason}")]
    Malformed {
        kind: ObjectKind,
        reason: &'static str,
    },
    /// The text is not a 40-digit hex sha1.
    #[error("not a sha1 object id")]
    BadId,
}
