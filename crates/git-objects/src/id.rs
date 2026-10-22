//! Object ids and kinds: what a store is asked for, and what it must return.

use core::fmt;

use crate::ObjectError;

/// A sha1 object id.
#[derive(Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ObjectId([u8; 20]);

impl ObjectId {
    /// The id from its 20 raw bytes, as a tree entry holds it.
    pub fn from_bytes(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }

    /// The 20 raw bytes.
    pub fn as_bytes(&self) -> &[u8; 20] {
        &self.0
    }

    /// Reads 40 hex digits, either case.
    pub fn from_hex(text: &str) -> Result<Self, ObjectError> {
        let text = text.as_bytes();
        if text.len() != 40 {
            return Err(ObjectError::BadId);
        }
        let mut out = [0u8; 20];
        for (byte, pair) in out.iter_mut().zip(text.as_chunks::<2>().0) {
            *byte = (digit(pair[0])? << 4) | digit(pair[1])?;
        }
        Ok(Self(out))
    }
}

fn digit(c: u8) -> Result<u8, ObjectError> {
    match c {
        b'0'..=b'9' => Ok(c - b'0'),
        b'a'..=b'f' => Ok(c - b'a' + 10),
        b'A'..=b'F' => Ok(c - b'A' + 10),
        _ => Err(ObjectError::BadId),
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.iter().try_for_each(|b| write!(f, "{b:02x}"))
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ObjectId({self})")
    }
}

/// The four git object types.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ObjectKind {
    Blob,
    Tree,
    Commit,
    Tag,
}

impl ObjectKind {
    /// The name git writes in an object header.
    pub fn name(self) -> &'static str {
        match self {
            Self::Blob => "blob",
            Self::Tree => "tree",
            Self::Commit => "commit",
            Self::Tag => "tag",
        }
    }

    /// The kind a header name stands for, or `None` for any other name.
    pub fn from_name(name: &[u8]) -> Option<Self> {
        match name {
            b"blob" => Some(Self::Blob),
            b"tree" => Some(Self::Tree),
            b"commit" => Some(Self::Commit),
            b"tag" => Some(Self::Tag),
            _ => None,
        }
    }
}

impl fmt::Display for ObjectKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}
