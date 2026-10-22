//! What a page shows, decided from the objects alone: the view model the DOM
//! code draws.

use git_objects::{ObjectId, ObjectKind, TreeEntry};

use crate::object::{Error, Fetch, load};
use crate::{Layout, PageKind};

/// One line of a directory listing.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub name: String,
    /// Where it links, or none for a submodule, which no store here holds.
    pub href: Option<String>,
    pub is_dir: bool,
}

/// What fills the page's content area.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    /// A directory.
    Tree(Vec<Row>),
    /// A file that is text.
    Text(String),
    /// A file that is not, with its size in bytes.
    Binary(usize),
    /// An object on the way, or the file itself, could not be had.
    Unheld,
    /// The commit and trees are held, and the path is not in them.
    NoSuchPath,
}

fn percent_encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// The page address of `segments` below the layout's ref.
fn address(layout: &Layout, page: &str, segments: &[&str]) -> String {
    let mut href = format!("{}/{page}", layout.base);
    for segment in layout.ref_name.split('/').chain(segments.iter().copied()) {
        href.push('/');
        href.push_str(&percent_encode(segment));
    }
    href
}

fn rows(layout: &Layout, entries: &[TreeEntry]) -> Vec<Row> {
    let directory: Vec<&str> = layout.path.iter().map(String::as_str).collect();
    let mut rows: Vec<Row> = entries
        .iter()
        .map(|entry| {
            let mut segments = directory.clone();
            segments.push(&entry.name);
            let is_dir = entry.is_tree();
            Row {
                name: entry.name.clone(),
                href: (!entry.is_gitlink())
                    .then(|| address(layout, if is_dir { "tree" } else { "blob" }, &segments)),
                is_dir,
            }
        })
        .collect();
    rows.sort_by(|a, b| (!a.is_dir, &a.name).cmp(&(!b.is_dir, &b.name)));
    rows
}

/// A file's content as the page shows it.
fn file(bytes: &[u8]) -> Content {
    match core::str::from_utf8(bytes) {
        Ok(text) if !text.contains('\0') => Content::Text(text.to_owned()),
        _ => Content::Binary(bytes.len()),
    }
}

/// Walks from the commit to what the layout's path names, fetching each
/// object on the way.
pub async fn build(fetch: &impl Fetch, layout: &Layout) -> Content {
    walk(fetch, layout).await.unwrap_or(Content::Unheld)
}

async fn walk(fetch: &impl Fetch, layout: &Layout) -> Result<Content, Error> {
    let id = |text: &str| ObjectId::from_hex(text).map_err(|_| Error::Unheld);
    let commit = load(fetch, layout, id(&layout.commit)?, ObjectKind::Commit).await?;
    let mut tree_id = commit.parse_commit().map_err(|_| Error::Unheld)?.tree;
    let (last, directories) = match (layout.kind, layout.path.split_last()) {
        (PageKind::Blob, Some((last, directories))) => (Some(last), directories),
        _ => (None, layout.path.as_slice()),
    };
    for segment in directories {
        let tree = load(fetch, layout, tree_id, ObjectKind::Tree).await?;
        let entries = tree.parse_tree().map_err(|_| Error::Unheld)?.entries;
        match entries
            .iter()
            .find(|entry| entry.name == *segment && entry.is_tree())
        {
            Some(entry) => tree_id = entry.id,
            None => return Ok(Content::NoSuchPath),
        }
    }
    let tree = load(fetch, layout, tree_id, ObjectKind::Tree).await?;
    let entries = tree.parse_tree().map_err(|_| Error::Unheld)?.entries;
    let Some(last) = last else {
        return Ok(Content::Tree(rows(layout, &entries)));
    };
    let Some(entry) = entries
        .iter()
        .find(|entry| entry.name == *last && !entry.is_tree() && !entry.is_gitlink())
    else {
        return Ok(Content::NoSuchPath);
    };
    let blob = load(fetch, layout, entry.id, ObjectKind::Blob).await?;
    Ok(file(blob.blob().map_err(|_| Error::Unheld)?))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use git_objects::hash_object;

    use super::*;
    use crate::testing::Table;

    /// Raw tree body of `(mode, name, id)` entries.
    fn tree(entries: &[(&str, &str, ObjectId)]) -> Vec<u8> {
        let mut body = Vec::new();
        for (mode, name, id) in entries {
            body.extend_from_slice(format!("{mode} {name}\0").as_bytes());
            body.extend_from_slice(id.as_bytes());
        }
        body
    }

    struct Repo {
        table: Table,
        objects: BTreeMap<String, Vec<String>>,
        commit: ObjectId,
    }

    impl Repo {
        fn add(&mut self, kind: ObjectKind, body: Vec<u8>) -> ObjectId {
            let id = hash_object(kind, &body);
            let url = format!("https://objects.test/{id}");
            self.objects.insert(id.to_string(), vec![url.clone()]);
            self.table.0.insert(url, body);
            id
        }

        fn layout(&self, kind: PageKind, path: &[&str]) -> Layout {
            Layout {
                kind,
                base: "/repo/npub1x/proj".into(),
                ref_name: "feature/x".into(),
                commit: self.commit.to_string(),
                path: path.iter().map(|s| (*s).to_owned()).collect(),
                objects: self.objects.clone(),
            }
        }
    }

    /// `README` and `src/main.rs`, plus a binary and a submodule.
    fn repo() -> (Repo, ObjectId, ObjectId) {
        let mut repo = Repo {
            table: Table(BTreeMap::new()),
            objects: BTreeMap::new(),
            commit: ObjectId::from_bytes([0; 20]),
        };
        let readme = repo.add(ObjectKind::Blob, b"read me\n".to_vec());
        let main = repo.add(ObjectKind::Blob, b"fn main() {}\n".to_vec());
        let binary = repo.add(ObjectKind::Blob, vec![0, 1, 2]);
        let src = repo.add(ObjectKind::Tree, tree(&[("100644", "main.rs", main)]));
        let root = repo.add(
            ObjectKind::Tree,
            tree(&[
                ("100644", "README", readme),
                ("100644", "data.bin", binary),
                ("40000", "src", src),
                ("160000", "vendor", ObjectId::from_bytes([9; 20])),
            ]),
        );
        let commit =
            format!("tree {root}\nauthor A <a@x> 1 +0000\ncommitter A <a@x> 1 +0000\n\nmsg\n");
        repo.commit = repo.add(ObjectKind::Commit, commit.into_bytes());
        (repo, readme, main)
    }

    #[tokio::test]
    async fn a_tree_lists_directories_first_and_links_by_ref_and_path() {
        let (repo, ..) = repo();
        let content = build(&repo.table, &repo.layout(PageKind::Tree, &[])).await;
        let Content::Tree(rows) = content else {
            panic!("{content:?}")
        };
        let names: Vec<_> = rows.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["src", "README", "data.bin", "vendor"]);
        assert_eq!(
            rows[0].href.as_deref(),
            Some("/repo/npub1x/proj/tree/feature/x/src")
        );
        assert_eq!(
            rows[1].href.as_deref(),
            Some("/repo/npub1x/proj/blob/feature/x/README")
        );
        assert_eq!(rows[3].href, None);
        let sub = build(&repo.table, &repo.layout(PageKind::Tree, &["src"])).await;
        let Content::Tree(rows) = sub else { panic!() };
        assert_eq!(
            rows[0].href.as_deref(),
            Some("/repo/npub1x/proj/blob/feature/x/src/main.rs")
        );
    }

    #[tokio::test]
    async fn a_blob_is_text_or_binary() {
        let (repo, ..) = repo();
        let text = build(
            &repo.table,
            &repo.layout(PageKind::Blob, &["src", "main.rs"]),
        )
        .await;
        assert_eq!(text, Content::Text("fn main() {}\n".into()));
        let binary = build(&repo.table, &repo.layout(PageKind::Blob, &["data.bin"])).await;
        assert_eq!(binary, Content::Binary(3));
    }

    #[tokio::test]
    async fn a_path_that_is_not_there_is_not_an_unheld_object() {
        let (repo, ..) = repo();
        for (kind, path) in [
            (PageKind::Blob, vec!["nope"]),
            (PageKind::Blob, vec!["src"]),
            (PageKind::Tree, vec!["nope"]),
            (PageKind::Blob, vec!["vendor"]),
        ] {
            let content = build(&repo.table, &repo.layout(kind, &path)).await;
            assert_eq!(content, Content::NoSuchPath, "{path:?}");
        }
    }

    #[tokio::test]
    async fn a_missing_object_is_unheld_and_its_siblings_still_render() {
        let (mut repo, readme, main) = repo();
        for gone in [readme, main] {
            repo.table.0.remove(&format!("https://objects.test/{gone}"));
        }
        let missing = build(&repo.table, &repo.layout(PageKind::Blob, &["README"])).await;
        assert_eq!(missing, Content::Unheld);
        // The tree listing never needed the blobs.
        let listing = build(&repo.table, &repo.layout(PageKind::Tree, &[])).await;
        assert!(matches!(listing, Content::Tree(_)));
        // A different file is unaffected.
        let other = build(&repo.table, &repo.layout(PageKind::Blob, &["data.bin"])).await;
        assert_eq!(other, Content::Binary(3));
    }
}
