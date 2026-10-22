//! What the server wrote into the page's `data-git` attribute.

use std::collections::BTreeMap;

use serde::Deserialize;

/// Which page: a directory listing or a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PageKind {
    Tree,
    Blob,
}

/// The page's layout.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct Layout {
    pub kind: PageKind,
    /// The repo's path, `/repo/<npub>/<d>`.
    pub base: String,
    /// The ref as the visitor named it.
    #[serde(rename = "ref")]
    pub ref_name: String,
    /// The commit the ref resolved to, 40 hex digits.
    pub commit: String,
    /// The path below the ref, one entry per segment.
    pub path: Vec<String>,
    /// Object id to its candidate URLs, in the order to try them.
    pub objects: BTreeMap<String, Vec<String>>,
}
