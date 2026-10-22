//! Walking a commit's ancestry through a caller-supplied fetch, so the
//! history view needs no I/O from this crate.

use core::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, HashSet};

use crate::{Commit, ObjectError, ObjectId, RawObject};

/// One step of a walk: the commit, or why it could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WalkedCommit {
    pub id: ObjectId,
    pub commit: Result<Commit, ObjectError>,
}

/// Walks from `start` through parents, each commit once, up to `max`
/// commits, newest committer time first (ties in the order found).
///
/// `fetch` returns the object for an id, verified by
/// [`read_loose`](crate::read_loose) or [`verify_body`](crate::verify_body),
/// or the reason it is not usable. A commit is fetched when it is first found,
/// so it can be ordered by its own time. A commit that cannot be fetched or
/// read is yielded as an error, ordered at the time of the child that found
/// it, and the walk goes on with the rest, since a hole in one branch does not
/// hide another.
pub fn walk_history<F>(start: ObjectId, max: usize, mut fetch: F) -> Vec<WalkedCommit>
where
    F: FnMut(&ObjectId) -> Result<RawObject, ObjectError>,
{
    // Max-heap on (time, reverse discovery order), so ties keep discovery order.
    let mut queue = BinaryHeap::new();
    let mut found: HashMap<ObjectId, Result<Commit, ObjectError>> = HashMap::new();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let mut discover = |id: ObjectId, child_time: i64| {
        let commit = fetch(&id).and_then(|object| object.parse_commit());
        // An unreadable commit has no time of its own; it takes its child's,
        // which puts it next.
        let time = commit.as_ref().map_or(child_time, |c| c.committer.time);
        (time, commit)
    };

    if max == 0 {
        return out;
    }
    let (time, commit) = discover(start, i64::MAX);
    seen.insert(start);
    found.insert(start, commit);
    queue.push((time, Reverse(0usize), start));

    while let Some((_, _, id)) = queue.pop() {
        let Some(commit) = found.remove(&id) else {
            continue;
        };
        if let Ok(commit) = &commit {
            for parent in &commit.parents {
                if seen.insert(*parent) {
                    let (time, parent_commit) = discover(*parent, commit.committer.time);
                    found.insert(*parent, parent_commit);
                    queue.push((time, Reverse(seen.len()), *parent));
                }
            }
        }
        out.push(WalkedCommit { id, commit });
        if out.len() >= max {
            break;
        }
    }
    out
}
