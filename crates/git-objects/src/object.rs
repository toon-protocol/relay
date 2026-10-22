//! Parsing verified object bodies into typed values.

use crate::{ObjectError, ObjectId, ObjectKind};

/// An object's kind and its body (no header). A blob's body is its content.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawObject {
    pub kind: ObjectKind,
    pub body: Vec<u8>,
}

impl RawObject {
    fn expect(&self, expected: ObjectKind) -> Result<&[u8], ObjectError> {
        if self.kind == expected {
            Ok(&self.body)
        } else {
            Err(ObjectError::WrongKind {
                expected,
                found: self.kind,
            })
        }
    }

    pub fn parse_tree(&self) -> Result<Tree, ObjectError> {
        Tree::parse(self.expect(ObjectKind::Tree)?)
    }

    pub fn parse_commit(&self) -> Result<Commit, ObjectError> {
        Commit::parse(self.expect(ObjectKind::Commit)?)
    }

    pub fn parse_tag(&self) -> Result<Tag, ObjectError> {
        Tag::parse(self.expect(ObjectKind::Tag)?)
    }

    /// A blob's content.
    pub fn blob(&self) -> Result<&[u8], ObjectError> {
        self.expect(ObjectKind::Blob)
    }
}

/// One line of a tree: a name bound to an object.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    /// The octal mode as git writes it: `100644`, `40000`, `120000`, `160000`.
    pub mode: String,
    pub name: String,
    pub id: ObjectId,
}

impl TreeEntry {
    pub fn is_tree(&self) -> bool {
        self.mode == "40000"
    }

    /// A submodule commit, which no store here holds.
    pub fn is_gitlink(&self) -> bool {
        self.mode == "160000"
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tree {
    pub entries: Vec<TreeEntry>,
}

fn malformed(kind: ObjectKind, reason: &'static str) -> ObjectError {
    ObjectError::Malformed { kind, reason }
}

impl Tree {
    pub fn parse(mut body: &[u8]) -> Result<Self, ObjectError> {
        const KIND: ObjectKind = ObjectKind::Tree;
        let mut entries = Vec::new();
        while !body.is_empty() {
            let space = body
                .iter()
                .position(|&b| b == b' ')
                .ok_or(malformed(KIND, "entry has no mode"))?;
            let mode = &body[..space];
            if mode.is_empty() || !mode.iter().all(|b| (b'0'..=b'7').contains(b)) {
                return Err(malformed(KIND, "entry mode is not octal"));
            }
            let rest = &body[space + 1..];
            let nul = rest
                .iter()
                .position(|&b| b == 0)
                .ok_or(malformed(KIND, "entry has no end of name"))?;
            let name = &rest[..nul];
            let after = &rest[nul + 1..];
            let id: [u8; 20] = after
                .get(..20)
                .and_then(|s| s.try_into().ok())
                .ok_or(malformed(KIND, "entry id is truncated"))?;
            entries.push(TreeEntry {
                mode: String::from_utf8_lossy(mode).into_owned(),
                name: String::from_utf8_lossy(name).into_owned(),
                id: ObjectId::from_bytes(id),
            });
            body = &after[20..];
        }
        Ok(Self { entries })
    }
}

/// An author, committer or tagger line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Person {
    pub name: String,
    pub email: String,
    /// Seconds since the Unix epoch.
    pub time: i64,
    /// The offset as written, `+0100`.
    pub tz: String,
}

impl Person {
    fn parse(kind: ObjectKind, line: &str) -> Result<Self, ObjectError> {
        let bad = || malformed(kind, "person line is unreadable");
        let open = line.find('<').ok_or_else(bad)?;
        let close = line.rfind('>').ok_or_else(bad)?;
        if close < open {
            return Err(bad());
        }
        let name = line[..open].trim_end().to_owned();
        let email = line[open + 1..close].to_owned();
        let mut tail = line[close + 1..].split_whitespace();
        let time = tail.next().and_then(|t| t.parse().ok()).ok_or_else(bad)?;
        let tz = tail.next().ok_or_else(bad)?.to_owned();
        Ok(Self {
            name,
            email,
            time,
            tz,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Commit {
    pub tree: ObjectId,
    /// Empty for a root commit, two or more for a merge.
    pub parents: Vec<ObjectId>,
    pub author: Person,
    pub committer: Person,
    pub message: String,
}

/// Splits a commit or tag body into its header lines (continuations joined
/// to the line they continue) and its message.
fn split_headers(body: &[u8]) -> (Vec<(String, String)>, String) {
    let text = String::from_utf8_lossy(body);
    let (head, message) = match text.split_once("\n\n") {
        Some((head, message)) => (head, message.to_owned()),
        None => (text.trim_end_matches('\n'), String::new()),
    };
    let mut headers: Vec<(String, String)> = Vec::new();
    for line in head.split('\n') {
        if let Some(cont) = line.strip_prefix(' ') {
            if let Some((_, value)) = headers.last_mut() {
                value.push('\n');
                value.push_str(cont);
            }
        } else {
            let (key, value) = line.split_once(' ').unwrap_or((line, ""));
            headers.push((key.to_owned(), value.to_owned()));
        }
    }
    (headers, message)
}

fn first<'a>(headers: &'a [(String, String)], key: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.as_str())
}

fn id_header(
    kind: ObjectKind,
    headers: &[(String, String)],
    key: &str,
    reason: &'static str,
) -> Result<ObjectId, ObjectError> {
    first(headers, key)
        .and_then(|v| ObjectId::from_hex(v).ok())
        .ok_or(malformed(kind, reason))
}

impl Commit {
    pub fn parse(body: &[u8]) -> Result<Self, ObjectError> {
        const KIND: ObjectKind = ObjectKind::Commit;
        let (headers, message) = split_headers(body);
        let tree = id_header(KIND, &headers, "tree", "no readable tree line")?;
        let parents = headers
            .iter()
            .filter(|(k, _)| k == "parent")
            .map(|(_, v)| ObjectId::from_hex(v))
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| malformed(KIND, "parent line is not an id"))?;
        let person = |key, reason| {
            first(&headers, key)
                .ok_or(malformed(KIND, reason))
                .and_then(|line| Person::parse(KIND, line))
        };
        Ok(Self {
            tree,
            parents,
            author: person("author", "no author line")?,
            committer: person("committer", "no committer line")?,
            message,
        })
    }

    pub fn is_merge(&self) -> bool {
        self.parents.len() > 1
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub object: ObjectId,
    pub target_kind: ObjectKind,
    pub name: String,
    /// Old tags have none.
    pub tagger: Option<Person>,
    pub message: String,
}

impl Tag {
    pub fn parse(body: &[u8]) -> Result<Self, ObjectError> {
        const KIND: ObjectKind = ObjectKind::Tag;
        let (headers, message) = split_headers(body);
        let object = id_header(KIND, &headers, "object", "no readable object line")?;
        let target_kind = first(&headers, "type")
            .and_then(|t| ObjectKind::from_name(t.as_bytes()))
            .ok_or(malformed(KIND, "no readable type line"))?;
        let name = first(&headers, "tag")
            .ok_or(malformed(KIND, "no tag line"))?
            .to_owned();
        let tagger = first(&headers, "tagger")
            .map(|line| Person::parse(KIND, line))
            .transpose()?;
        Ok(Self {
            object,
            target_kind,
            name,
            tagger,
            message,
        })
    }
}
