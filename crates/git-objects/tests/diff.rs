//! Diffing a commit's files: renames, deletions and what changed in a file.

use git_objects::{FileChange, FileEntry, LineOp, ObjectId, diff_files, diff_lines};

fn file(path: &str, byte: u8) -> FileEntry {
    FileEntry {
        path: path.to_owned(),
        mode: "100644".to_owned(),
        id: ObjectId::from_bytes([byte; 20]),
    }
}

#[test]
fn a_rename_is_one_change_with_both_paths() {
    let old = [file("docs/a.md", 1), file("keep.txt", 2)];
    let new = [file("guide/a.md", 1)];
    let changes = diff_files(&old, &new);
    assert_eq!(
        changes,
        [
            FileChange::Renamed {
                old: file("docs/a.md", 1),
                new: file("guide/a.md", 1)
            },
            FileChange::Deleted(file("keep.txt", 2)),
        ]
    );
}

#[test]
fn a_deletion_and_an_addition_of_different_content_stay_apart() {
    // Only an exact content match is a rename, so a moved and edited file is
    // a deletion and an addition.
    let changes = diff_files(&[file("old.txt", 1)], &[file("new.txt", 2)]);
    assert_eq!(
        changes,
        [
            FileChange::Added(file("new.txt", 2)),
            FileChange::Deleted(file("old.txt", 1)),
        ]
    );
}

#[test]
fn identical_copies_pair_one_to_one_in_path_order() {
    let old = [file("a", 7), file("b", 7)];
    let new = [file("c", 7), file("d", 7), file("e", 7)];
    let changes = diff_files(&old, &new);
    assert_eq!(changes.len(), 3);
    assert_eq!(
        changes[0],
        FileChange::Renamed {
            old: file("a", 7),
            new: file("c", 7)
        }
    );
    assert_eq!(
        changes[1],
        FileChange::Renamed {
            old: file("b", 7),
            new: file("d", 7)
        }
    );
    assert_eq!(changes[2], FileChange::Added(file("e", 7)));
}

#[test]
fn a_changed_id_or_mode_at_one_path_is_modified_and_a_same_file_is_not_listed() {
    let mut executable = file("run.sh", 1);
    executable.mode = "100755".to_owned();
    let changes = diff_files(
        &[file("run.sh", 1), file("same", 3), file("edit", 4)],
        &[executable.clone(), file("same", 3), file("edit", 5)],
    );
    assert_eq!(
        changes,
        [
            FileChange::Modified {
                old: file("edit", 4),
                new: file("edit", 5)
            },
            FileChange::Modified {
                old: file("run.sh", 1),
                new: executable
            },
        ]
    );
}

#[test]
fn deleting_every_line_is_all_deletes() {
    let lines = diff_lines(b"one\ntwo\n", b"");
    assert!(lines.iter().all(|line| line.op == LineOp::Delete));
    assert_eq!(lines.len(), 2);
}

#[test]
fn an_edited_line_is_a_delete_then_an_insert_between_equal_lines() {
    let lines = diff_lines(b"a\nb\nc\n", b"a\nB\nc\n");
    let ops: Vec<_> = lines.iter().map(|l| (l.op, l.text.as_str())).collect();
    assert_eq!(
        ops,
        [
            (LineOp::Equal, "a"),
            (LineOp::Delete, "b"),
            (LineOp::Insert, "B"),
            (LineOp::Equal, "c"),
        ]
    );
}
