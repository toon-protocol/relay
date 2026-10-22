//! Diffing trees and blobs.

use similar::{ChangeTag, TextDiff};

use crate::{Tree, TreeEntry};

/// What happened to one name between two trees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    Added(TreeEntry),
    Removed(TreeEntry),
    /// Same name, different id or mode.
    Modified {
        old: TreeEntry,
        new: TreeEntry,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeChange {
    pub name: String,
    pub change: Change,
}

/// Compares two trees one level deep, in name order. A changed subtree shows
/// as `Modified`; the caller fetches both sides and calls this again.
pub fn diff_trees(old: &Tree, new: &Tree) -> Vec<TreeChange> {
    let mut old_entries: Vec<&TreeEntry> = old.entries.iter().collect();
    let mut new_entries: Vec<&TreeEntry> = new.entries.iter().collect();
    old_entries.sort_by(|a, b| a.name.cmp(&b.name));
    new_entries.sort_by(|a, b| a.name.cmp(&b.name));

    let (mut i, mut j) = (0, 0);
    let mut out = Vec::new();
    let mut push = |name: &str, change| {
        out.push(TreeChange {
            name: name.to_owned(),
            change,
        })
    };
    while i < old_entries.len() || j < new_entries.len() {
        match (old_entries.get(i), new_entries.get(j)) {
            (Some(o), Some(n)) if o.name == n.name => {
                if o.id != n.id || o.mode != n.mode {
                    push(
                        &o.name,
                        Change::Modified {
                            old: (*o).clone(),
                            new: (*n).clone(),
                        },
                    );
                }
                i += 1;
                j += 1;
            }
            (Some(o), Some(n)) if o.name < n.name => {
                push(&o.name, Change::Removed((*o).clone()));
                i += 1;
            }
            (Some(_), Some(n)) => {
                push(&n.name, Change::Added((*n).clone()));
                j += 1;
            }
            (Some(o), None) => {
                push(&o.name, Change::Removed((*o).clone()));
                i += 1;
            }
            (None, Some(n)) => {
                push(&n.name, Change::Added((*n).clone()));
                j += 1;
            }
            (None, None) => break,
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineOp {
    Equal,
    Delete,
    Insert,
}

/// One line of a line diff, without its newline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LineChange {
    pub op: LineOp,
    pub text: String,
}

/// A line diff of two blobs' contents (lossily read as UTF-8).
pub fn diff_lines(old: &[u8], new: &[u8]) -> Vec<LineChange> {
    let (old, new) = (String::from_utf8_lossy(old), String::from_utf8_lossy(new));
    TextDiff::from_lines(old.as_ref(), new.as_ref())
        .iter_all_changes()
        .map(|change| LineChange {
            op: match change.tag() {
                ChangeTag::Equal => LineOp::Equal,
                ChangeTag::Delete => LineOp::Delete,
                ChangeTag::Insert => LineOp::Insert,
            },
            text: change.value().trim_end_matches(['\n', '\r']).to_owned(),
        })
        .collect()
}
