//! Git objects as typed values, from bytes alone.
//!
//! The `/repo` WASM module reads objects out of stores (Arweave, rig) that hold
//! them without a trust relationship, so every object is hashed against the
//! id it was asked for before it is parsed. This crate does that and the
//! parsing, with no I/O and no async: a caller supplies bytes, or a fetch
//! callback for [`walk_history`]. It compiles for `wasm32-unknown-unknown`
//! and natively.
//!
//! A mismatched, oversized or missing object is an [`ObjectError`] value,
//! never a panic; the UI shows each as "no longer held".

mod diff;
mod error;
mod id;
mod object;
mod verify;
mod walk;

pub use diff::{
    Change, FileChange, FileEntry, LineChange, LineOp, TreeChange, diff_files, diff_lines,
    diff_trees,
};
pub use error::ObjectError;
pub use id::{ObjectId, ObjectKind};
pub use object::{Commit, Person, RawObject, Tag, Tree, TreeEntry};
pub use verify::{Limits, hash_object, read_loose, verify_body};
pub use walk::{WalkedCommit, walk_history};
