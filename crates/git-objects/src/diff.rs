//! Diffing trees and blobs, for the `/repo` view of what a commit changed.

use std::collections::BTreeMap;

use similar::{ChangeTag, TextDiff};

use crate::{ObjectId, Tree, TreeEntry};

/// What happened to one name between two trees.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Change {
    /// Only in the new tree.
    Added(TreeEntry),
    /// Only in the old tree.
    Removed(TreeEntry),
    /// Same name, different id or mode.
    Modified { old: TreeEntry, new: TreeEntry },
}

/// A change to the entry called `name`.
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

/// Whether a line is in both blobs, only the old one, or only the new one.
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

/// One file (not a directory) at a path from the root of a tree.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileEntry {
    /// Slash-separated, from the root of the tree.
    pub path: String,
    pub mode: String,
    pub id: ObjectId,
}

/// What a commit did to one file, with a file that moved paired with its old
/// path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileChange {
    Added(FileEntry),
    Deleted(FileEntry),
    /// Same path, different id or mode.
    Modified {
        old: FileEntry,
        new: FileEntry,
    },
    /// A deleted and an added file with the same content id. The paths differ.
    Renamed {
        old: FileEntry,
        new: FileEntry,
    },
}

impl FileChange {
    /// The path the change is listed under: the new one, or the old one for a
    /// deletion.
    pub fn path(&self) -> &str {
        match self {
            Self::Added(file) | Self::Deleted(file) => &file.path,
            Self::Modified { new, .. } | Self::Renamed { new, .. } => &new.path,
        }
    }
}

/// Compares two flat lists of files by path. A file that left one path and
/// appears at another with the same content id is a rename (each file pairs
/// at most once, in path order); changes come back in path order. The lists
/// may leave out files that are the same on both sides.
pub fn diff_files(old: &[FileEntry], new: &[FileEntry]) -> Vec<FileChange> {
    let old: BTreeMap<&str, &FileEntry> = old.iter().map(|f| (f.path.as_str(), f)).collect();
    let new: BTreeMap<&str, &FileEntry> = new.iter().map(|f| (f.path.as_str(), f)).collect();
    let mut out = Vec::new();
    let mut deleted: Vec<&FileEntry> = Vec::new();
    for (path, o) in &old {
        match new.get(path) {
            Some(n) if n.id != o.id || n.mode != o.mode => out.push(FileChange::Modified {
                old: (*o).clone(),
                new: (*n).clone(),
            }),
            Some(_) => {}
            None => deleted.push(o),
        }
    }
    for (path, n) in &new {
        if old.contains_key(path) {
            continue;
        }
        match deleted.iter().position(|o| o.id == n.id) {
            Some(index) => out.push(FileChange::Renamed {
                old: deleted.remove(index).clone(),
                new: (*n).clone(),
            }),
            None => out.push(FileChange::Added((*n).clone())),
        }
    }
    out.extend(deleted.into_iter().map(|o| FileChange::Deleted(o.clone())));
    out.sort_by(|a, b| a.path().cmp(b.path()));
    out
}
