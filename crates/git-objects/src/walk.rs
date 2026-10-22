//! Walking a commit's ancestry through a caller-supplied fetch.

use std::collections::{BinaryHeap, HashSet};

use crate::{Commit, ObjectError, ObjectId, ObjectKind, RawObject};

/// One step of a walk: the commit, or why it could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkedCommit {
    pub id: ObjectId,
    pub commit: Result<Commit, ObjectError>,
}

/// Walks from `start` through parents, each
/// commit once, up to `max` commits. A parent is ordered by the committer time
/// of the child that reached it, so history reads newest-first.
///
/// `fetch` returns the object for an id, already verified (see
/// [`read_loose`](crate::read_loose)), or the reason it is not usable. A
/// commit that cannot be fetched or read is yielded as an error and the walk
/// goes on with the rest, since a hole in one branch does not hide another.
pub fn walk_history<F>(start: ObjectId, max: usize, mut fetch: F) -> Vec<WalkedCommit>
where
    F: FnMut(&ObjectId) -> Result<RawObject, ObjectError>,
{
    // Max-heap on (time, reverse discovery order) so ties keep discovery order.
    let mut queue: BinaryHeap<(i64, core::cmp::Reverse<usize>, ObjectId)> = BinaryHeap::new();
    let mut seen = HashSet::from([start]);
    let mut discovered = 1usize;
    let mut out = Vec::new();
    // The start has no known time yet; it is the only entry, so any works.
    queue.push((i64::MAX, core::cmp::Reverse(0), start));

    while let Some((_, _, id)) = queue.pop() {
        if out.len() >= max {
            break;
        }
        let commit = fetch(&id).and_then(|object| {
            if object.kind == ObjectKind::Commit {
                object.parse_commit()
            } else {
                Err(ObjectError::WrongKind {
                    expected: ObjectKind::Commit,
                    found: object.kind,
                })
            }
        });
        if let Ok(commit) = &commit {
            for parent in &commit.parents {
                if seen.insert(*parent) {
                    // A parent is queued by its child's time until it is read;
                    // never later than the child, so the order stays newest-first.
                    queue.push((
                        commit.committer.time,
                        core::cmp::Reverse(discovered),
                        *parent,
                    ));
                    discovered += 1;
                }
            }
        }
        out.push(WalkedCommit { id, commit });
    }
    out
}
