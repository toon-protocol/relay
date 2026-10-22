//! Hashing an object against the id it was asked for.

use std::io::Read;

use flate2::read::ZlibDecoder;
use sha1::{Digest, Sha1};

use crate::{ObjectError, ObjectId, ObjectKind, RawObject};

/// Bounds on what a caller will accept.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The largest object body, in bytes.
    pub max_body: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_body: 32 * 1024 * 1024,
        }
    }
}

/// The id of an object of `kind` with `body` (the bytes after the header).
pub fn hash_object(kind: ObjectKind, body: &[u8]) -> ObjectId {
    let mut hasher = Sha1::new();
    hasher.update(kind.name().as_bytes());
    hasher.update(b" ");
    hasher.update(body.len().to_string().as_bytes());
    hasher.update([0u8]);
    hasher.update(body);
    ObjectId::from_bytes(hasher.finalize().into())
}

/// Checks a header-less `body` of `kind` against `expected`, the form
/// Arweave and rig stores hold.
pub fn verify_body(
    expected: ObjectId,
    kind: ObjectKind,
    body: Vec<u8>,
    limits: Limits,
) -> Result<RawObject, ObjectError> {
    if body.len() as u64 > limits.max_body {
        return Err(ObjectError::Oversized {
            size: body.len() as u64,
            limit: limits.max_body,
        });
    }
    let actual = hash_object(kind, &body);
    if actual != expected {
        return Err(ObjectError::Mismatch { expected, actual });
    }
    Ok(RawObject { kind, body })
}

/// Inflates a zlib-compressed loose object (`<kind> <size>\0<body>`) and
/// checks it against `expected`. Inflation stops at the size limit, so a
/// decompression bomb costs `max_body` bytes at most.
pub fn read_loose(
    expected: ObjectId,
    compressed: &[u8],
    limits: Limits,
) -> Result<RawObject, ObjectError> {
    // Room for the header on top of the body.
    let cap = limits.max_body.saturating_add(64);
    let mut inflated = Vec::new();
    ZlibDecoder::new(compressed)
        .take(cap.saturating_add(1))
        .read_to_end(&mut inflated)
        .map_err(|_| ObjectError::BadLoose("not valid zlib data"))?;

    let nul = inflated
        .iter()
        .position(|&b| b == 0)
        .ok_or(ObjectError::BadLoose("no end of header"))?;
    let header = &inflated[..nul];
    let space = header
        .iter()
        .position(|&b| b == b' ')
        .ok_or(ObjectError::BadLoose("header has no size"))?;
    let kind = ObjectKind::from_name(&header[..space])
        .ok_or(ObjectError::BadLoose("unknown object type"))?;
    let size: u64 = core::str::from_utf8(&header[space + 1..])
        .ok()
        .filter(|s| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()))
        .and_then(|s| s.parse().ok())
        .ok_or(ObjectError::BadLoose("size is not a number"))?;
    if size > limits.max_body {
        return Err(ObjectError::Oversized {
            size,
            limit: limits.max_body,
        });
    }
    let body = inflated.split_off(nul + 1);
    if body.len() as u64 != size {
        return Err(ObjectError::BadLoose("size does not match the body"));
    }
    verify_body(expected, kind, body, limits)
}
