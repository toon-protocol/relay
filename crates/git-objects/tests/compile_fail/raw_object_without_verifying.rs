//! A `RawObject` is proof its bytes hashed to their id, so it cannot be built
//! from bytes that were never checked.

use git_objects::{ObjectKind, RawObject};

fn main() {
    let _ = RawObject {
        kind: ObjectKind::Blob,
        body: b"unchecked".to_vec(),
    };
}
