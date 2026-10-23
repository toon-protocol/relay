//! What a page shows, decided from the objects alone: the view model the DOM
//! code draws.

use git_objects::{ObjectId, ObjectKind, TreeEntry};
use ui_markdown::Node;

use crate::history::{self, CommitView, Log, commit_href, when};
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

/// The commit a listing is at, for the bar above it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Head {
    pub id: ObjectId,
    pub href: String,
    /// The first line of the message.
    pub summary: String,
    /// The author's name alone.
    pub author: String,
    pub date: String,
}

/// A directory's README, as the listing shows it below the files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Readme {
    pub name: String,
    pub body: Document,
}

/// A text file as a page reads it: Markdown is drawn, anything else is not.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Document {
    Markdown(Vec<Node>),
    Plain(String),
    /// The README is named but could not be had.
    Unheld,
}

/// A directory: its entries, the commit, and its README if it has one.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listing {
    pub rows: Vec<Row>,
    pub head: Head,
    pub readme: Option<Readme>,
}

/// What fills the page's content area.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    /// A directory.
    Tree(Listing),
    /// A file that is text, and its Markdown drawn if it is Markdown.
    Text {
        text: String,
        markdown: Option<Vec<Node>>,
    },
    /// A file that is not, with its size in bytes.
    Binary(usize),
    /// An object on the way, or the file itself, could not be had.
    Unheld,
    /// One page of the commit log.
    Log(Log),
    /// One commit and its changes.
    Commit(Box<CommitView>),
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
pub(crate) fn address(layout: &Layout, page: &str, segments: &[&str]) -> String {
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

fn is_markdown(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [".md", ".markdown", ".mdown", ".mkd"]
        .iter()
        .any(|extension| lower.ends_with(extension))
}

/// Whether a directory entry is the README a listing shows.
fn is_readme(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "readme",
        "readme.md",
        "readme.markdown",
        "readme.txt",
        "readme.rst",
    ]
    .contains(&lower.as_str())
}

/// Where a link in a document in `directory` goes: a page of this repo at
/// the same ref for a relative path, the address itself for an absolute
/// one, and nowhere for any other scheme.
fn document_link(layout: &Layout, directory: &[String], url: &str) -> Option<String> {
    if url.starts_with('#') || ui_markdown::scheme(url).is_some() {
        return ui_markdown::absolute_only(url);
    }
    if url.starts_with("//") {
        return None;
    }
    let path = url.split(['#', '?']).next().unwrap_or_default();
    let mut segments: Vec<&str> = if path.starts_with('/') {
        Vec::new()
    } else {
        directory.iter().map(String::as_str).collect()
    };
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            other => segments.push(other),
        }
    }
    let page = if path.ends_with('/') || segments.is_empty() {
        "tree"
    } else {
        "blob"
    };
    Some(address(layout, page, &segments))
}

/// `text` read as a document in `directory`.
fn document(layout: &Layout, directory: &[String], name: &str, text: &str) -> Document {
    if is_markdown(name) {
        Document::Markdown(ui_markdown::parse(text, |url| {
            document_link(layout, directory, url)
        }))
    } else {
        Document::Plain(text.to_owned())
    }
}

fn text_of(bytes: &[u8]) -> Option<&str> {
    core::str::from_utf8(bytes)
        .ok()
        .filter(|text| !text.contains('\0'))
}

/// A file's content as the page shows it.
fn file(layout: &Layout, bytes: &[u8]) -> Content {
    match text_of(bytes) {
        Some(text) => {
            let (name, directory) = layout
                .path
                .split_last()
                .map_or(("", &[][..]), |(name, directory)| {
                    (name.as_str(), directory)
                });
            let markdown = match document(layout, directory, name, text) {
                Document::Markdown(nodes) => Some(nodes),
                Document::Plain(_) | Document::Unheld => None,
            };
            Content::Text {
                text: text.to_owned(),
                markdown,
            }
        }
        None => Content::Binary(bytes.len()),
    }
}

/// The README among `entries`, fetched and read, if there is one.
async fn readme(fetch: &impl Fetch, layout: &Layout, entries: &[TreeEntry]) -> Option<Readme> {
    let entry = entries
        .iter()
        .filter(|entry| !entry.is_tree() && !entry.is_gitlink() && is_readme(&entry.name))
        .min_by_key(|entry| !is_markdown(&entry.name))?;
    let body = match load(fetch, layout, entry.id, ObjectKind::Blob).await {
        Ok(object) => match object.blob().ok().and_then(text_of) {
            Some(text) => document(layout, &layout.path, &entry.name, text),
            None => Document::Unheld,
        },
        Err(_) => Document::Unheld,
    };
    Some(Readme {
        name: entry.name.clone(),
        body,
    })
}

/// Walks from the commit to what the layout's path names, fetching each
/// object on the way.
pub async fn build(fetch: &impl Fetch, layout: &Layout) -> Content {
    walk(fetch, layout).await.unwrap_or(Content::Unheld)
}

async fn walk(fetch: &impl Fetch, layout: &Layout) -> Result<Content, Error> {
    match layout.kind {
        PageKind::Commits => return history::log(fetch, layout).await.map(Content::Log),
        PageKind::Commit => {
            return history::commit(fetch, layout)
                .await
                .map(|view| Content::Commit(Box::new(view)));
        }
        PageKind::Tree | PageKind::Blob => {}
    }
    let id = |text: &str| ObjectId::from_hex(text).map_err(|_| Error::Unheld);
    let commit_id = id(&layout.commit)?;
    let commit = load(fetch, layout, commit_id, ObjectKind::Commit).await?;
    let commit = commit.parse_commit().map_err(|_| Error::Unheld)?;
    let head = Head {
        id: commit_id,
        href: commit_href(layout, &commit_id),
        summary: commit.message.lines().next().unwrap_or_default().to_owned(),
        author: commit.author.name.clone(),
        date: when(&commit.author),
    };
    let mut tree_id = commit.tree;
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
        return Ok(Content::Tree(Listing {
            rows: rows(layout, &entries),
            head,
            readme: readme(fetch, layout, &entries).await,
        }));
    };
    let Some(entry) = entries
        .iter()
        .find(|entry| entry.name == *last && !entry.is_tree() && !entry.is_gitlink())
    else {
        return Ok(Content::NoSuchPath);
    };
    let blob = load(fetch, layout, entry.id, ObjectKind::Blob).await?;
    Ok(file(layout, blob.blob().map_err(|_| Error::Unheld)?))
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
                page: 0,
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
        let Content::Tree(Listing { rows, head, readme }) = content else {
            panic!("{content:?}")
        };
        assert_eq!(head.summary, "msg");
        assert_eq!(head.author, "A");
        let readme = readme.expect("README is listed");
        assert_eq!(
            (readme.name.as_str(), readme.body),
            ("README", Document::Plain("read me\n".into()))
        );
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
        let Content::Tree(Listing { rows, readme, .. }) = sub else {
            panic!()
        };
        assert_eq!(readme, None);
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
        assert_eq!(
            text,
            Content::Text {
                text: "fn main() {}\n".into(),
                markdown: None
            }
        );
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
        // The tree listing still renders; only its README is not held.
        let listing = build(&repo.table, &repo.layout(PageKind::Tree, &[])).await;
        let Content::Tree(listing) = listing else {
            panic!("{listing:?}")
        };
        assert_eq!(listing.rows.len(), 4);
        assert_eq!(listing.readme.unwrap().body, Document::Unheld);
        // A different file is unaffected.
        let other = build(&repo.table, &repo.layout(PageKind::Blob, &["data.bin"])).await;
        assert_eq!(other, Content::Binary(3));
    }

    #[test]
    fn a_documents_relative_links_stay_in_the_repo_at_its_ref() {
        let (repo, ..) = repo();
        let layout = repo.layout(PageKind::Blob, &["docs", "guide.md"]);
        let directory = ["docs".to_owned()];
        let at = |url: &str| document_link(&layout, &directory, url);
        let base = "/repo/npub1x/proj";
        assert_eq!(
            at("a.md").as_deref(),
            Some(&*format!("{base}/blob/feature/x/docs/a.md"))
        );
        assert_eq!(
            at("../README.md#usage").as_deref(),
            Some(&*format!("{base}/blob/feature/x/README.md"))
        );
        assert_eq!(
            at("/src/").as_deref(),
            Some(&*format!("{base}/tree/feature/x/src"))
        );
        assert_eq!(at("https://x.test/").as_deref(), Some("https://x.test/"));
        assert_eq!(at("#top").as_deref(), Some("#top"));
        assert_eq!(at("javascript:alert(1)"), None);
        assert_eq!(at("//elsewhere.test/x"), None);
    }

    #[tokio::test]
    async fn a_markdown_file_is_drawn_as_well_as_shown() {
        let (mut repo, ..) = repo();
        let doc = repo.add(ObjectKind::Blob, b"# Hi\n\n[next](b.md)\n".to_vec());
        let root = repo.add(ObjectKind::Tree, tree(&[("100644", "a.md", doc)]));
        let commit =
            format!("tree {root}\nauthor A <a@x> 1 +0000\ncommitter A <a@x> 1 +0000\n\nmsg\n");
        repo.commit = repo.add(ObjectKind::Commit, commit.into_bytes());
        let content = build(&repo.table, &repo.layout(PageKind::Blob, &["a.md"])).await;
        let Content::Text {
            text,
            markdown: Some(nodes),
        } = content
        else {
            panic!("{content:?}")
        };
        assert_eq!(text, "# Hi\n\n[next](b.md)\n");
        assert_eq!(
            ui_markdown::to_html(&nodes),
            "<h1>Hi</h1><p><a href=\"/repo/npub1x/proj/blob/feature/x/b.md\" \
             rel=\"nofollow noopener\">next</a></p>"
        );
    }
}
