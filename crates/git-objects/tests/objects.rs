//! The crate against real git objects kept as files in `tests/fixtures`.

use std::collections::HashMap;
use std::fs;

use git_objects::{
    Change, Limits, LineOp, ObjectError, ObjectId, ObjectKind, RawObject, diff_lines, diff_trees,
    hash_object, read_loose, verify_body, walk_history,
};

const FIRST: &str = "9e0fd459c253d978b6c23c64977c6f17aa0e1830";
const SIDE: &str = "e0c3d4c0f261453fcc52913426ed7b2e2a10d5c9";
const MAIN_CHANGE: &str = "8b9d303e1c5b403d9eb5eb00dabcafd77f766d00";
const MERGE: &str = "06eda202e7f8a2dacb5c42fd4c3ae5a2d006accd";
const TAG: &str = "5c64559b6f697c535a86c9180df1107a334026ea";
const MERGE_TREE: &str = "7ad712b79a6add1c4833d8291d84aeb0dae61e5f";
const FIRST_TREE: &str = "b88029b736582d066ea154f0d4167f6c474b5ab0";
const BLOB_OLD: &str = "94954abda49de8615a048f8d2e64b5de848e27a1";
const BLOB_NEW: &str = "363f0a506d5d243463c5ee27404c269dd063dc29";

fn id(hex: &str) -> ObjectId {
    ObjectId::from_hex(hex).unwrap()
}

fn fixture(hex: &str, ext: &str) -> Vec<u8> {
    fs::read(format!("tests/fixtures/{hex}.{ext}")).unwrap()
}

fn kind_of(hex: &str) -> ObjectKind {
    let name = fixture(hex, "type");
    ObjectKind::from_name(name.trim_ascii()).unwrap()
}

fn object(hex: &str) -> RawObject {
    read_loose(id(hex), &fixture(hex, "loose"), Limits::default()).unwrap()
}

#[test]
fn every_fixture_verifies_both_as_loose_and_as_a_headerless_body() {
    let mut kinds = Vec::new();
    for entry in fs::read_dir("tests/fixtures").unwrap() {
        let name = entry.unwrap().file_name().into_string().unwrap();
        let Some(hex) = name.strip_suffix(".loose") else {
            continue;
        };
        let loose = object(hex);
        let body = verify_body(
            id(hex),
            kind_of(hex),
            fixture(hex, "body"),
            Limits::default(),
        )
        .unwrap();
        assert_eq!(loose, body, "{hex}");
        kinds.push(loose.kind);
    }
    for kind in [
        ObjectKind::Blob,
        ObjectKind::Tree,
        ObjectKind::Commit,
        ObjectKind::Tag,
    ] {
        assert!(kinds.contains(&kind), "no {kind} fixture");
    }
}

#[test]
fn a_body_that_does_not_hash_to_its_id_is_a_mismatch() {
    let mut body = fixture(BLOB_OLD, "body");
    body.push(b'!');
    let err = verify_body(
        id(BLOB_OLD),
        ObjectKind::Blob,
        body.clone(),
        Limits::default(),
    )
    .unwrap_err();
    assert_eq!(
        err,
        ObjectError::Mismatch {
            expected: id(BLOB_OLD),
            actual: hash_object(ObjectKind::Blob, &body),
        }
    );
}

#[test]
fn a_loose_object_served_under_another_id_is_a_mismatch() {
    let err = read_loose(id(BLOB_NEW), &fixture(BLOB_OLD, "loose"), Limits::default()).unwrap_err();
    assert!(matches!(err, ObjectError::Mismatch { .. }), "{err:?}");
}

#[test]
fn the_right_bytes_under_the_wrong_kind_are_a_mismatch() {
    let err = verify_body(
        id(BLOB_OLD),
        ObjectKind::Tree,
        fixture(BLOB_OLD, "body"),
        Limits::default(),
    )
    .unwrap_err();
    assert!(matches!(err, ObjectError::Mismatch { .. }));
}

#[test]
fn an_object_over_the_limit_is_oversized_in_either_form() {
    let limits = Limits { max_body: 3 };
    let expect = ObjectError::Oversized { size: 12, limit: 3 };
    assert_eq!(
        verify_body(
            id(BLOB_OLD),
            ObjectKind::Blob,
            fixture(BLOB_OLD, "body"),
            limits
        )
        .unwrap_err(),
        expect
    );
    assert_eq!(
        read_loose(id(BLOB_OLD), &fixture(BLOB_OLD, "loose"), limits).unwrap_err(),
        expect
    );
}

#[test]
fn garbage_is_a_bad_loose_object_not_a_panic() {
    for bytes in [
        &b""[..],
        b"not zlib at all",
        &fixture(BLOB_OLD, "loose")[..10],
    ] {
        let err = read_loose(id(BLOB_OLD), bytes, Limits::default()).unwrap_err();
        assert!(matches!(err, ObjectError::BadLoose(_)), "{err:?}");
    }
}

#[test]
fn ids_round_trip_through_hex_and_reject_other_text() {
    assert_eq!(id(FIRST).to_string(), FIRST);
    assert_eq!(
        ObjectId::from_hex(&FIRST.to_uppercase()).unwrap(),
        id(FIRST)
    );
    for bad in ["", "abc", &FIRST[..39], &format!("{}g", &FIRST[..39])] {
        assert_eq!(ObjectId::from_hex(bad), Err(ObjectError::BadId), "{bad}");
    }
}

#[test]
fn a_blob_is_its_content() {
    assert_eq!(object(BLOB_OLD).blob().unwrap(), b"hello\nworld\n");
}

#[test]
fn a_tree_lists_its_entries_with_modes() {
    let tree = object(MERGE_TREE).parse_tree().unwrap();
    let listing: Vec<_> = tree
        .entries
        .iter()
        .map(|e| (e.mode.as_str(), e.name.as_str(), e.is_tree()))
        .collect();
    assert_eq!(
        listing,
        [
            ("100644", "a.txt", false),
            ("100644", "side.txt", false),
            ("40000", "src", true)
        ]
    );
    assert_eq!(tree.entries[0].id, id(BLOB_NEW));
}

#[test]
fn a_root_commit_has_no_parents() {
    let commit = object(FIRST).parse_commit().unwrap();
    assert!(commit.parents.is_empty());
    assert_eq!(commit.tree, id(FIRST_TREE));
    assert_eq!(commit.message, "first\n");
    assert_eq!(commit.author.name, "Fixture");
    assert_eq!(commit.author.email, "f@example.com");
    assert_eq!(commit.author.time, 1_767_225_600);
    assert_eq!(commit.author.tz, "+0000");
}

#[test]
fn a_merge_commit_has_both_parents_in_order() {
    let commit = object(MERGE).parse_commit().unwrap();
    assert!(commit.is_merge());
    assert_eq!(commit.parents, [id(MAIN_CHANGE), id(SIDE)]);
    assert_eq!(commit.tree, id(MERGE_TREE));
    assert_eq!(commit.message, "merge side\n");
}

#[test]
fn an_annotated_tag_names_its_target() {
    let tag = object(TAG).parse_tag().unwrap();
    assert_eq!(tag.object, id(MERGE));
    assert_eq!(tag.target_kind, ObjectKind::Commit);
    assert_eq!(tag.name, "v1");
    assert_eq!(tag.tagger.unwrap().name, "Fixture");
    assert_eq!(tag.message, "release one\n");
}

#[test]
fn parsing_as_the_wrong_kind_says_so() {
    let err = object(BLOB_OLD).parse_commit().unwrap_err();
    assert_eq!(
        err,
        ObjectError::WrongKind {
            expected: ObjectKind::Commit,
            found: ObjectKind::Blob
        }
    );
}

#[test]
fn a_signature_header_continues_across_lines_and_does_not_hide_the_message() {
    let body = format!(
        "tree {FIRST_TREE}\nauthor A <a@b> 1 +0100\ncommitter A <a@b> 1 +0100\n\
         gpgsig -----BEGIN-----\n line\n -----END-----\n\nsigned\n"
    );
    let commit = git_objects::Commit::parse(body.as_bytes()).unwrap();
    assert_eq!(commit.message, "signed\n");
    assert_eq!(commit.author.tz, "+0100");
}

#[test]
fn malformed_bodies_are_typed_errors() {
    assert!(matches!(
        git_objects::Commit::parse(b"author nobody\n\nmsg"),
        Err(ObjectError::Malformed { .. })
    ));
    assert!(matches!(
        git_objects::Tree::parse(b"100644 a\0short"),
        Err(ObjectError::Malformed { .. })
    ));
    assert!(matches!(
        git_objects::Tag::parse(b""),
        Err(ObjectError::Malformed { .. })
    ));
}

fn store() -> HashMap<ObjectId, RawObject> {
    fs::read_dir("tests/fixtures")
        .unwrap()
        .filter_map(|e| {
            let name = e.unwrap().file_name().into_string().unwrap();
            name.strip_suffix(".loose").map(|h| (id(h), object(h)))
        })
        .collect()
}

#[test]
fn walking_a_merge_visits_every_commit_once_newest_first() {
    let store = store();
    let walked = walk_history(id(MERGE), 100, |i| {
        store.get(i).cloned().ok_or(ObjectError::Missing(*i))
    });
    let ids: Vec<_> = walked.iter().map(|w| w.id).collect();
    assert_eq!(ids, [id(MERGE), id(MAIN_CHANGE), id(SIDE), id(FIRST)]);
    assert!(walked.iter().all(|w| w.commit.is_ok()));
}

#[test]
fn walking_stops_at_the_requested_count() {
    let store = store();
    let walked = walk_history(id(MERGE), 2, |i| {
        store.get(i).cloned().ok_or(ObjectError::Missing(*i))
    });
    assert_eq!(walked.len(), 2);
}

#[test]
fn a_missing_commit_is_reported_and_the_other_branch_is_still_walked() {
    let mut store = store();
    store.remove(&id(SIDE));
    let walked = walk_history(id(MERGE), 100, |i| {
        store.get(i).cloned().ok_or(ObjectError::Missing(*i))
    });
    let missing: Vec<_> = walked.iter().filter(|w| w.commit.is_err()).collect();
    assert_eq!(missing.len(), 1);
    assert_eq!(missing[0].id, id(SIDE));
    assert_eq!(missing[0].commit, Err(ObjectError::Missing(id(SIDE))));
    assert!(walked.iter().any(|w| w.id == id(FIRST) && w.commit.is_ok()));
}

#[test]
fn walking_onto_a_non_commit_is_a_wrong_kind_outcome() {
    let store = store();
    let walked = walk_history(id(BLOB_OLD), 10, |i| {
        store.get(i).cloned().ok_or(ObjectError::Missing(*i))
    });
    assert!(matches!(
        walked[0].commit,
        Err(ObjectError::WrongKind { .. })
    ));
}

#[test]
fn diffing_trees_reports_added_modified_and_removed_names() {
    let first = object(FIRST_TREE).parse_tree().unwrap();
    let merged = object(MERGE_TREE).parse_tree().unwrap();
    let changes: Vec<_> = diff_trees(&first, &merged)
        .into_iter()
        .map(|c| {
            let kind = match c.change {
                Change::Added(_) => "added",
                Change::Removed(_) => "removed",
                Change::Modified { .. } => "modified",
            };
            (c.name, kind)
        })
        .collect();
    assert_eq!(
        changes,
        [("a.txt".into(), "modified"), ("side.txt".into(), "added")]
    );

    let back: Vec<_> = diff_trees(&merged, &first)
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(back, ["a.txt", "side.txt"]);
    assert!(matches!(
        diff_trees(&merged, &first)[1].change,
        Change::Removed(_)
    ));
    assert!(diff_trees(&merged, &merged).is_empty());
}

#[test]
fn a_line_diff_marks_the_inserted_line_between_two_blobs() {
    let old = object(BLOB_OLD);
    let new = object(BLOB_NEW);
    let lines: Vec<_> = diff_lines(old.blob().unwrap(), new.blob().unwrap())
        .into_iter()
        .map(|l| (l.op, l.text))
        .collect();
    assert_eq!(
        lines,
        [
            (LineOp::Equal, "hello".to_owned()),
            (LineOp::Insert, "there".to_owned()),
            (LineOp::Equal, "world".to_owned()),
        ]
    );
}
