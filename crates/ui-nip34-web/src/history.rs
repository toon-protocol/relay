//! The commit log and commit pages (#286): history walked from a ref, and a
//! commit's changes against its first parent, from fetched objects alone.

use std::collections::HashMap;

use git_objects::{
    Change, Commit, FileChange, FileEntry, LineOp, ObjectError, ObjectId, ObjectKind, RawObject,
    Tree, diff_files, diff_lines, diff_trees, walk_history,
};

use crate::Layout;
use crate::object::{Error, Fetch, load};
use crate::view::address;

/// Commits on one page of the log.
pub const PAGE_SIZE: usize = 20;

/// What a log says where the walk met an object nobody holds.
pub const HISTORY_ENDS: &str = "history beyond this point is no longer held";

/// Directories read, and files compared, before a commit page gives up. A
/// commit that touches more is not one a page can show.
const MAX_DIRECTORIES: usize = 300;
const MAX_FILES: usize = 500;

/// A file's diff longer than this many lines is not shown.
const MAX_DIFF_LINES: usize = 5_000;

/// Unchanged lines kept either side of a change.
const CONTEXT: usize = 3;

/// One commit in the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LogRow {
    pub href: String,
    pub id: ObjectId,
    pub summary: String,
    pub author: String,
    pub date: String,
}

/// One page of the log.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Log {
    pub rows: Vec<LogRow>,
    /// The address of the page of newer commits, if this is not the first.
    pub newer: Option<String>,
    /// The address of the page of older commits, if there are more.
    pub older: Option<String>,
    /// The walk met a missing object where this page ends.
    pub ended: bool,
}

/// A link to another commit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitLink {
    pub id: ObjectId,
    pub href: String,
}

/// What one file's change shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Body {
    /// A unified-style diff: `+`, `-` or a space before each line, and
    /// `@@` lines where unchanged lines are left out.
    Diff(String),
    Binary,
    /// A blob of this change could not be had.
    Unheld,
    /// Something to say in place of a diff: a submodule, a mode change, a
    /// pure rename, a diff too long.
    Note(&'static str),
}

/// One changed file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileView {
    /// `added`, `deleted`, `modified` or `renamed`.
    pub status: &'static str,
    /// The path, or `old -> new` for a rename.
    pub label: String,
    pub body: Body,
}

/// The changes against the first parent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Files {
    /// The parent, or a tree on the way, could not be had.
    Unheld,
    /// More than a page can show.
    TooMany,
    Changed(Vec<FileView>),
}

/// A commit page.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommitView {
    pub id: ObjectId,
    pub author: String,
    pub date: String,
    pub message: String,
    pub parents: Vec<CommitLink>,
    pub tree_href: String,
    pub files: Files,
}

fn id_of(text: &str) -> Result<ObjectId, Error> {
    ObjectId::from_hex(text).map_err(|_| Error::Unheld)
}

fn commit_href(layout: &Layout, id: &ObjectId) -> String {
    format!("{}/commit/{id}", layout.base)
}

fn tree_href(layout: &Layout, id: &ObjectId) -> String {
    format!("{}/tree/{id}", layout.base)
}

/// `Name <email>`, or what is there of it.
fn who(person: &git_objects::Person) -> String {
    if person.email.is_empty() {
        person.name.clone()
    } else {
        format!("{} <{}>", person.name, person.email)
    }
}

/// `YYYY-MM-DD HH:MM +ZZZZ` in the person's own time zone.
fn when(person: &git_objects::Person) -> String {
    let offset = person
        .tz
        .strip_prefix(['+', '-'])
        .filter(|digits| digits.len() == 4 && digits.bytes().all(|b| b.is_ascii_digit()))
        .map_or(0, |digits| {
            let minutes =
                digits[..2].parse::<i64>().unwrap_or(0) * 60 + digits[2..].parse().unwrap_or(0);
            if person.tz.starts_with('-') {
                -minutes
            } else {
                minutes
            }
        });
    let local = person.time.saturating_add(offset * 60);
    let (days, rest) = (local.div_euclid(86_400), local.rem_euclid(86_400));
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted_month + 2) / 5 + 1;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    let zone = if person.tz.is_empty() {
        "+0000"
    } else {
        &person.tz
    };
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02} {zone}",
        rest / 3_600,
        rest % 3_600 / 60
    )
}

/// Walks from `start`, fetching what the walk asks for. The walk itself is
/// synchronous, so it runs again after each round of fetches, from a cache,
/// until it asks for nothing new.
async fn walk(
    fetch: &impl Fetch,
    layout: &Layout,
    start: ObjectId,
    max: usize,
) -> Vec<git_objects::WalkedCommit> {
    let mut cache: HashMap<ObjectId, Result<RawObject, ObjectError>> = HashMap::new();
    loop {
        let mut wanted: Vec<ObjectId> = Vec::new();
        let walked = walk_history(start, max, |id| match cache.get(id) {
            Some(held) => held.clone(),
            None => {
                if !wanted.contains(id) {
                    wanted.push(*id);
                }
                Err(ObjectError::Missing(*id))
            }
        });
        if wanted.is_empty() {
            return walked;
        }
        for id in wanted {
            let held = load(fetch, layout, id, ObjectKind::Commit)
                .await
                .map_err(|_| ObjectError::Missing(id));
            cache.insert(id, held);
        }
    }
}

fn page_address(layout: &Layout, page: usize) -> String {
    let base = address(layout, "commits", &[]);
    if page == 0 {
        base
    } else {
        format!("{base}?page={page}")
    }
}

/// One page of the log from the layout's commit.
pub async fn log(fetch: &impl Fetch, layout: &Layout) -> Result<Log, Error> {
    let skip = layout.page.saturating_mul(PAGE_SIZE);
    let walked = walk(fetch, layout, id_of(&layout.commit)?, skip + PAGE_SIZE + 1).await;
    // The log ends at the first object nobody holds.
    let held = walked
        .iter()
        .position(|step| step.commit.is_err())
        .unwrap_or(walked.len());
    if held == 0 {
        return Err(Error::Unheld);
    }
    let rows = walked[..held]
        .iter()
        .skip(skip)
        .take(PAGE_SIZE)
        .filter_map(|step| {
            let commit = step.commit.as_ref().ok()?;
            Some(LogRow {
                href: commit_href(layout, &step.id),
                id: step.id,
                summary: commit.message.lines().next().unwrap_or_default().to_owned(),
                author: who(&commit.author),
                date: when(&commit.committer),
            })
        })
        .collect();
    Ok(Log {
        rows,
        newer: (layout.page > 0).then(|| page_address(layout, layout.page - 1)),
        older: (held > skip + PAGE_SIZE).then(|| page_address(layout, layout.page + 1)),
        ended: held < walked.len() && held <= skip + PAGE_SIZE,
    })
}

async fn tree_entries(fetch: &impl Fetch, layout: &Layout, id: ObjectId) -> Result<Tree, Error> {
    let tree = load(fetch, layout, id, ObjectKind::Tree).await?;
    tree.parse_tree().map_err(|_| Error::Unheld)
}

fn path_in(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}/{name}")
    }
}

/// Every file below the changed directories, on each side: only what differs
/// between two trees is read, so a commit that touches one file reads the
/// trees on its path.
async fn changed_files(
    fetch: &impl Fetch,
    layout: &Layout,
    old: Option<ObjectId>,
    new: ObjectId,
) -> Result<Result<(Vec<FileEntry>, Vec<FileEntry>), ()>, Error> {
    // (prefix, old tree, new tree)
    let mut work = vec![(String::new(), old, Some(new))];
    let (mut old_files, mut new_files) = (Vec::new(), Vec::new());
    let mut directories = 0;
    while let Some((prefix, old, new)) = work.pop() {
        directories += 1;
        if directories > MAX_DIRECTORIES || old_files.len() + new_files.len() > MAX_FILES {
            return Ok(Err(()));
        }
        let empty = Tree { entries: vec![] };
        let old = match old {
            Some(id) => tree_entries(fetch, layout, id).await?,
            None => empty.clone(),
        };
        let new = match new {
            Some(id) => tree_entries(fetch, layout, id).await?,
            None => empty,
        };
        for entry in diff_trees(&old, &new) {
            let path = path_in(&prefix, &entry.name);
            // One side of a change: a directory to read whole, or a file.
            let mut side = |entry: git_objects::TreeEntry, is_new: bool| {
                if entry.is_tree() {
                    let id = Some(entry.id);
                    work.push(if is_new {
                        (path.clone(), None, id)
                    } else {
                        (path.clone(), id, None)
                    });
                } else {
                    let file = FileEntry {
                        path: path.clone(),
                        mode: entry.mode,
                        id: entry.id,
                    };
                    if is_new {
                        &mut new_files
                    } else {
                        &mut old_files
                    }
                    .push(file);
                }
            };
            match entry.change {
                Change::Added(new) => side(new, true),
                Change::Removed(old) => side(old, false),
                Change::Modified { old, new } if old.is_tree() && new.is_tree() => {
                    work.push((path, Some(old.id), Some(new.id)));
                }
                Change::Modified { old, new } => {
                    side(old, false);
                    side(new, true);
                }
            }
        }
    }
    if old_files.len() + new_files.len() > MAX_FILES {
        return Ok(Err(()));
    }
    Ok(Ok((old_files, new_files)))
}

fn is_text(bytes: &[u8]) -> bool {
    core::str::from_utf8(bytes).is_ok_and(|text| !text.contains('\0'))
}

/// A diff as text, with `CONTEXT` unchanged lines around each change.
fn unified(old: &[u8], new: &[u8]) -> Option<String> {
    let lines = diff_lines(old, new);
    if lines.len() > MAX_DIFF_LINES {
        return None;
    }
    let near_change = |index: usize| {
        let from = index.saturating_sub(CONTEXT);
        lines[from..lines.len().min(index + CONTEXT + 1)]
            .iter()
            .any(|line| line.op != LineOp::Equal)
    };
    let mut out = String::new();
    let mut skipped = 0;
    let flush = |out: &mut String, skipped: &mut usize| {
        if *skipped > 0 {
            out.push_str(&format!("@@ {skipped} unchanged lines @@\n"));
            *skipped = 0;
        }
    };
    for (index, line) in lines.iter().enumerate() {
        if line.op == LineOp::Equal && !near_change(index) {
            skipped += 1;
            continue;
        }
        flush(&mut out, &mut skipped);
        out.push(match line.op {
            LineOp::Equal => ' ',
            LineOp::Delete => '-',
            LineOp::Insert => '+',
        });
        out.push_str(&line.text);
        out.push('\n');
    }
    flush(&mut out, &mut skipped);
    Some(out)
}

/// A blob's bytes, empty for none.
async fn blob(
    fetch: &impl Fetch,
    layout: &Layout,
    file: Option<&FileEntry>,
) -> Result<Vec<u8>, ()> {
    match file {
        None => Ok(Vec::new()),
        Some(file) => {
            let object = load(fetch, layout, file.id, ObjectKind::Blob)
                .await
                .map_err(|_| ())?;
            object.blob().map(<[u8]>::to_vec).map_err(|_| ())
        }
    }
}

async fn file_view(fetch: &impl Fetch, layout: &Layout, change: &FileChange) -> FileView {
    let (status, label, old, new) = match change {
        FileChange::Added(file) => ("added", file.path.clone(), None, Some(file)),
        FileChange::Deleted(file) => ("deleted", file.path.clone(), Some(file), None),
        FileChange::Modified { old, new } => ("modified", new.path.clone(), Some(old), Some(new)),
        FileChange::Renamed { old, new } => (
            "renamed",
            format!("{} -> {}", old.path, new.path),
            Some(old),
            Some(new),
        ),
    };
    let view = |body| FileView {
        status,
        label: label.clone(),
        body,
    };
    if old
        .into_iter()
        .chain(new)
        .any(|file| file.mode.trim_start_matches('0') == "160000")
    {
        return view(Body::Note("submodule"));
    }
    if let (Some(old), Some(new)) = (old, new)
        && old.id == new.id
    {
        return view(Body::Note(if old.path == new.path {
            "mode changed"
        } else {
            "renamed without changes"
        }));
    }
    let (Ok(old), Ok(new)) = (
        blob(fetch, layout, old).await,
        blob(fetch, layout, new).await,
    ) else {
        return view(Body::Unheld);
    };
    if !is_text(&old) || !is_text(&new) {
        return view(Body::Binary);
    }
    view(match unified(&old, &new) {
        Some(diff) => Body::Diff(diff),
        None => Body::Note("diff too long to show"),
    })
}

async fn files(fetch: &impl Fetch, layout: &Layout, commit: &Commit) -> Files {
    // Against the first parent, or against nothing for a root commit.
    let old = match commit.parents.first() {
        None => None,
        Some(parent) => {
            let Ok(object) = load(fetch, layout, *parent, ObjectKind::Commit).await else {
                return Files::Unheld;
            };
            let Ok(parent) = object.parse_commit() else {
                return Files::Unheld;
            };
            Some(parent.tree)
        }
    };
    let (old_files, new_files) = match changed_files(fetch, layout, old, commit.tree).await {
        Ok(Ok(sides)) => sides,
        Ok(Err(())) => return Files::TooMany,
        Err(_) => return Files::Unheld,
    };
    let mut views = Vec::new();
    for change in diff_files(&old_files, &new_files) {
        views.push(file_view(fetch, layout, &change).await);
    }
    Files::Changed(views)
}

/// The commit the layout names, with its changes.
pub async fn commit(fetch: &impl Fetch, layout: &Layout) -> Result<CommitView, Error> {
    let id = id_of(&layout.commit)?;
    let object = load(fetch, layout, id, ObjectKind::Commit).await?;
    let commit = object.parse_commit().map_err(|_| Error::Unheld)?;
    Ok(CommitView {
        id,
        author: who(&commit.author),
        date: when(&commit.author),
        message: commit.message.clone(),
        parents: commit
            .parents
            .iter()
            .map(|parent| CommitLink {
                id: *parent,
                href: commit_href(layout, parent),
            })
            .collect(),
        tree_href: tree_href(layout, &id),
        files: files(fetch, layout, &commit).await,
    })
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use git_objects::hash_object;

    use super::*;
    use crate::PageKind;
    use crate::testing::Table;

    fn tree(entries: &[(&str, &str, ObjectId)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (mode, name, id) in entries {
            body.extend_from_slice(format!("{mode} {name}\0").as_bytes());
            body.extend_from_slice(id.as_bytes());
        }
        body
    }

    #[derive(Default)]
    struct Repo {
        table: Table,
        objects: BTreeMap<String, Vec<String>>,
    }

    impl Repo {
        fn add(&mut self, kind: ObjectKind, body: Vec<u8>) -> ObjectId {
            let id = hash_object(kind, &body);
            let url = format!("https://objects.test/{id}");
            self.objects.insert(id.to_string(), vec![url.clone()]);
            self.table.0.insert(url, body);
            id
        }

        fn commit(
            &mut self,
            tree: ObjectId,
            parents: &[ObjectId],
            time: i64,
            msg: &str,
        ) -> ObjectId {
            let parents: String = parents.iter().map(|p| format!("parent {p}\n")).collect();
            let body = format!(
                "tree {tree}\n{parents}author Ada <a@x> {time} +0100\ncommitter Ada <a@x> {time} +0100\n\n{msg}\n"
            );
            self.add(ObjectKind::Commit, body.into_bytes())
        }

        fn forget(&mut self, id: ObjectId) {
            self.table.0.remove(&format!("https://objects.test/{id}"));
        }

        fn layout(&self, kind: PageKind, commit: ObjectId, page: usize) -> Layout {
            Layout {
                kind,
                base: "/repo/npub1x/proj".into(),
                ref_name: "main".into(),
                commit: commit.to_string(),
                path: vec![],
                page,
                objects: self.objects.clone(),
            }
        }

        /// `n` commits in a line over an empty tree, oldest first; the newest.
        fn chain(&mut self, n: usize) -> Vec<ObjectId> {
            let empty = self.add(ObjectKind::Tree, vec![]);
            let mut ids: Vec<ObjectId> = Vec::new();
            for index in 0..n {
                let parents: Vec<ObjectId> = ids.last().copied().into_iter().collect();
                let id = self.commit(
                    empty,
                    &parents,
                    1_000 + index as i64,
                    &format!("commit {index}"),
                );
                ids.push(id);
            }
            ids
        }
    }

    #[tokio::test]
    async fn the_log_is_paged_newest_first() {
        let mut repo = Repo::default();
        let ids = repo.chain(45);
        let tip = *ids.last().unwrap();
        let first = log(&repo.table, &repo.layout(PageKind::Commits, tip, 0))
            .await
            .unwrap();
        assert_eq!(first.rows.len(), PAGE_SIZE);
        assert_eq!(first.rows[0].summary, "commit 44");
        assert_eq!(
            first.rows[0].href,
            format!("/repo/npub1x/proj/commit/{tip}")
        );
        assert_eq!(first.newer, None);
        assert_eq!(
            first.older.as_deref(),
            Some("/repo/npub1x/proj/commits/main?page=1")
        );
        assert!(!first.ended);

        let last = log(&repo.table, &repo.layout(PageKind::Commits, tip, 2))
            .await
            .unwrap();
        assert_eq!(last.rows.len(), 5);
        assert_eq!(last.rows[4].summary, "commit 0");
        assert_eq!(
            last.newer.as_deref(),
            Some("/repo/npub1x/proj/commits/main?page=1")
        );
        assert_eq!(last.older, None);
        assert!(!last.ended, "reaching the root is not a hole");
    }

    #[tokio::test]
    async fn a_missing_commit_ends_the_log_with_a_marker() {
        let mut repo = Repo::default();
        let ids = repo.chain(5);
        repo.forget(ids[1]);
        let tip = ids[4];
        let page = log(&repo.table, &repo.layout(PageKind::Commits, tip, 0))
            .await
            .unwrap();
        let summaries: Vec<_> = page.rows.iter().map(|r| r.summary.as_str()).collect();
        assert_eq!(summaries, ["commit 4", "commit 3", "commit 2"]);
        assert!(page.ended);
        assert_eq!(page.older, None);
        // Dates are in the committer's zone.
        assert_eq!(page.rows[0].date, "1970-01-01 01:16 +0100");

        // A missing tip has no log at all.
        repo.forget(tip);
        let gone = log(&repo.table, &repo.layout(PageKind::Commits, tip, 0)).await;
        assert_eq!(gone.unwrap_err(), Error::Unheld);
    }

    #[tokio::test]
    async fn a_commit_shows_a_rename_a_deletion_and_an_edit() {
        let mut repo = Repo::default();
        let kept = repo.add(ObjectKind::Blob, b"keep\n".to_vec());
        let moved = repo.add(ObjectKind::Blob, b"moved\n".to_vec());
        let gone = repo.add(ObjectKind::Blob, b"bye\nnow\n".to_vec());
        let before = repo.add(ObjectKind::Blob, b"one\ntwo\nthree\n".to_vec());
        let after = repo.add(ObjectKind::Blob, b"one\n2\nthree\n".to_vec());
        let fresh = repo.add(ObjectKind::Blob, b"new\n".to_vec());
        let blob_bin = repo.add(ObjectKind::Blob, vec![0, 1]);
        let docs_old = repo.add(ObjectKind::Tree, tree(&[("100644", "moved.md", moved)]));
        let docs_new = repo.add(ObjectKind::Tree, tree(&[("100644", "new.txt", fresh)]));
        let guide = repo.add(ObjectKind::Tree, tree(&[("100644", "moved.md", moved)]));
        let old_root = repo.add(
            ObjectKind::Tree,
            tree(&[
                ("100644", "bin", blob_bin),
                ("100644", "edit.txt", before),
                ("100644", "gone.txt", gone),
                ("100644", "keep.txt", kept),
                ("40000", "old", docs_old),
            ]),
        );
        let new_root = repo.add(
            ObjectKind::Tree,
            tree(&[
                ("100644", "bin", blob_bin),
                ("100644", "edit.txt", after),
                ("40000", "guide", guide),
                ("100644", "keep.txt", kept),
                ("40000", "old", docs_new),
            ]),
        );
        let parent = repo.commit(old_root, &[], 1, "before");
        let child = repo.commit(new_root, &[parent], 2, "after");

        let view = commit(&repo.table, &repo.layout(PageKind::Commit, child, 0))
            .await
            .unwrap();
        assert_eq!(
            view.parents[0].href,
            format!("/repo/npub1x/proj/commit/{parent}")
        );
        assert_eq!(view.tree_href, format!("/repo/npub1x/proj/tree/{child}"));
        let Files::Changed(files) = view.files else {
            panic!("{:?}", view.files)
        };
        let listed: Vec<_> = files.iter().map(|f| (f.status, f.label.as_str())).collect();
        assert_eq!(
            listed,
            [
                ("modified", "edit.txt"),
                ("deleted", "gone.txt"),
                ("renamed", "old/moved.md -> guide/moved.md"),
                ("added", "old/new.txt"),
            ]
        );
        assert_eq!(files[0].body, Body::Diff(" one\n-two\n+2\n three\n".into()));
        assert_eq!(files[1].body, Body::Diff("-bye\n-now\n".into()));
        assert_eq!(files[2].body, Body::Note("renamed without changes"));
        assert_eq!(files[3].body, Body::Diff("+new\n".into()));

        // Losing a blob loses only its diff.
        repo.forget(after);
        let view = commit(&repo.table, &repo.layout(PageKind::Commit, child, 0))
            .await
            .unwrap();
        let Files::Changed(files) = view.files else {
            panic!()
        };
        assert_eq!(files[0].body, Body::Unheld);
        assert_eq!(files[1].body, Body::Diff("-bye\n-now\n".into()));
    }

    #[tokio::test]
    async fn a_root_commit_adds_its_files_and_a_missing_parent_is_unheld() {
        let mut repo = Repo::default();
        let file = repo.add(ObjectKind::Blob, b"a\n".to_vec());
        let binary = repo.add(ObjectKind::Blob, vec![0, 255]);
        let root = repo.add(
            ObjectKind::Tree,
            tree(&[
                ("100644", "a", file),
                ("100644", "b", binary),
                ("160000", "sub", ObjectId::from_bytes([9; 20])),
            ]),
        );
        let first = repo.commit(root, &[], 1, "root");
        let second = repo.commit(root, &[first], 2, "again");
        let view = commit(&repo.table, &repo.layout(PageKind::Commit, first, 0))
            .await
            .unwrap();
        assert!(view.parents.is_empty());
        let Files::Changed(files) = view.files else {
            panic!()
        };
        assert_eq!(files[0].body, Body::Diff("+a\n".into()));
        assert_eq!(files[1].body, Body::Binary);
        assert_eq!(files[2].body, Body::Note("submodule"));

        repo.forget(first);
        let view = commit(&repo.table, &repo.layout(PageKind::Commit, second, 0))
            .await
            .unwrap();
        assert_eq!(view.files, Files::Unheld);
    }

    #[tokio::test]
    async fn a_commit_over_the_file_limit_is_too_many_even_from_one_directory() {
        let mut repo = Repo::default();
        let names: Vec<String> = (0..=MAX_FILES).map(|n| format!("f{n}")).collect();
        let entries: Vec<_> = names
            .iter()
            .map(|name| ("100644", name.as_str(), ObjectId::from_bytes([1; 20])))
            .collect();
        let root = repo.add(ObjectKind::Tree, tree(&entries));
        let first = repo.commit(root, &[], 1, "many");
        let view = commit(&repo.table, &repo.layout(PageKind::Commit, first, 0))
            .await
            .unwrap();
        assert_eq!(view.files, Files::TooMany);
    }

    #[test]
    fn dates_follow_the_calendar() {
        let person = |time, tz: &str| git_objects::Person {
            name: String::new(),
            email: String::new(),
            time,
            tz: tz.to_owned(),
        };
        assert_eq!(
            when(&person(1_700_000_000, "+0000")),
            "2023-11-14 22:13 +0000"
        );
        assert_eq!(
            when(&person(1_709_164_800, "-0500")),
            "2024-02-28 19:00 -0500"
        );
        assert_eq!(
            when(&person(1_709_164_800, "+0000")),
            "2024-02-29 00:00 +0000"
        );
        assert_eq!(when(&person(0, "junk")), "1970-01-01 00:00 junk");
    }
}
