//! Getting one verified object out of its candidate URLs.

use std::future::Future;

use git_objects::{Limits, ObjectId, ObjectKind, RawObject, read_loose, verify_body};

use crate::Layout;

/// The most bytes an object may be (2 MiB).
pub const MAX_OBJECT: u64 = 2 * 1024 * 1024;

/// What a page says in place of an object it could not get.
pub const NO_LONGER_HELD: &str = "this object is no longer held";

/// Why a URL gave no bytes, or an object could not be had.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// One URL answered with more than the size limit.
    #[error("the object is over the 2 MiB limit")]
    Oversized,
    /// One URL was not found, refused, timed out or unreachable.
    #[error("the object could not be fetched")]
    Failed,
    /// The object is missing, oversized or not what its id says, from every
    /// place it was looked for.
    #[error("{NO_LONGER_HELD}")]
    Unheld,
}

/// Getting bytes from an `https` URL: at most `limit` bytes, with a timeout.
/// [`load`] only ever asks for `https` URLs. The browser implements it with
/// `fetch`; tests with a table.
pub trait Fetch {
    /// The bytes at `url`, or [`Error::Oversized`] past `limit`, or
    /// [`Error::Failed`].
    fn get(&self, url: &str, limit: u64) -> impl Future<Output = Result<Vec<u8>, Error>>;
}

const LIMITS: Limits = Limits {
    max_body: MAX_OBJECT,
};

/// The object `id` of `kind` from the first `https` candidate whose bytes
/// hash to `id`; any other URL is never fetched. The bytes may be the object's body (Arweave), the body behind its
/// `<kind> <size>\0` header (Blossom) or the zlib-compressed loose object;
/// the hash decides which, never the server.
pub async fn load(
    fetch: &impl Fetch,
    layout: &Layout,
    id: ObjectId,
    kind: ObjectKind,
) -> Result<RawObject, Error> {
    let candidates = layout.objects.get(&id.to_string()).ok_or(Error::Unheld)?;
    for url in candidates.iter().filter(|url| url.starts_with("https://")) {
        if let Ok(bytes) = fetch.get(url, MAX_OBJECT).await
            && let Some(object) = verified(id, kind, bytes)
        {
            return Ok(object);
        }
    }
    Err(Error::Unheld)
}

fn verified(id: ObjectId, kind: ObjectKind, bytes: Vec<u8>) -> Option<RawObject> {
    if bytes.len() as u64 > MAX_OBJECT {
        return None;
    }
    if let Some(body) = without_header(kind, &bytes)
        && let Ok(object) = verify_body(id, kind, body.to_vec(), LIMITS)
    {
        return Some(object);
    }
    if let Ok(object) = read_loose(id, &bytes, LIMITS)
        && object.kind() == kind
    {
        return Some(object);
    }
    verify_body(id, kind, bytes, LIMITS).ok()
}

/// The bytes after a `<kind> <size>\0` header whose size is the rest.
fn without_header(kind: ObjectKind, bytes: &[u8]) -> Option<&[u8]> {
    let rest = bytes
        .strip_prefix(kind.name().as_bytes())?
        .strip_prefix(b" ")?;
    let nul = rest.iter().position(|&b| b == 0)?;
    let size: usize = core::str::from_utf8(&rest[..nul]).ok()?.parse().ok()?;
    let body = &rest[nul + 1..];
    (body.len() == size).then_some(body)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::io::Write;

    use crate::testing::Table;

    use git_objects::hash_object;

    use super::*;

    fn layout(id: ObjectId, urls: &[&str]) -> Layout {
        Layout {
            kind: crate::PageKind::Blob,
            base: "/repo/x/y".into(),
            ref_name: "main".into(),
            commit: "0".repeat(40),
            path: vec![],
            page: 0,
            objects: BTreeMap::from([(
                id.to_string(),
                urls.iter().map(|u| (*u).to_owned()).collect(),
            )]),
        }
    }

    fn table(entries: &[(&str, Vec<u8>)]) -> Table {
        Table(
            entries
                .iter()
                .map(|(u, b)| ((*u).to_owned(), b.clone()))
                .collect(),
        )
    }

    const BODY: &[u8] = b"hello\n";

    fn id() -> ObjectId {
        hash_object(ObjectKind::Blob, BODY)
    }

    #[tokio::test]
    async fn the_three_formats_all_verify() {
        let mut headed = b"blob 6\0".to_vec();
        headed.extend_from_slice(BODY);
        let mut zlib = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        zlib.write_all(&headed).unwrap();
        let loose = zlib.finish().unwrap();
        for bytes in [BODY.to_vec(), headed, loose] {
            let fetch = table(&[("https://a.test/x", bytes)]);
            let object = load(
                &fetch,
                &layout(id(), &["https://a.test/x"]),
                id(),
                ObjectKind::Blob,
            )
            .await
            .unwrap();
            assert_eq!(object.blob().unwrap(), BODY);
        }
    }

    #[tokio::test]
    async fn candidates_are_tried_in_order_and_a_liar_is_skipped() {
        let fetch = table(&[
            ("https://liar.test/x", b"evil\n".to_vec()),
            ("https://honest.test/x", BODY.to_vec()),
            ("https://never.test/x", b"unreached".to_vec()),
        ]);
        let urls = [
            "https://gone.test/x",
            "http://insecure.test/x",
            "https://liar.test/x",
            "https://honest.test/x",
        ];
        let object = load(&fetch, &layout(id(), &urls), id(), ObjectKind::Blob).await;
        assert_eq!(object.unwrap().blob().unwrap(), BODY);
    }

    #[tokio::test]
    async fn a_missing_oversized_or_mismatched_object_is_unheld() {
        let big = vec![b'x'; MAX_OBJECT as usize + 1];
        let big_id = hash_object(ObjectKind::Blob, &big);
        let fetch = table(&[
            ("https://a.test/big", big),
            ("https://a.test/lie", b"lie".to_vec()),
        ]);
        for (id, url) in [
            (big_id, "https://a.test/big"),
            (id(), "https://a.test/lie"),
            (id(), "https://a.test/missing"),
        ] {
            let result = load(&fetch, &layout(id, &[url]), id, ObjectKind::Blob).await;
            assert_eq!(result.unwrap_err(), Error::Unheld, "{url}");
        }
        // An object the page lists no candidates for.
        let unlisted = layout(id(), &[]);
        assert_eq!(
            load(&fetch, &unlisted, big_id, ObjectKind::Blob)
                .await
                .unwrap_err(),
            Error::Unheld
        );
    }

    #[tokio::test]
    async fn an_object_held_only_over_plain_http_is_unheld() {
        let fetch = table(&[("http://insecure.test/x", BODY.to_vec())]);
        let result = load(
            &fetch,
            &layout(id(), &["http://insecure.test/x"]),
            id(),
            ObjectKind::Blob,
        )
        .await;
        assert_eq!(result.unwrap_err(), Error::Unheld);
    }

    #[tokio::test]
    async fn the_kind_asked_for_must_match() {
        let fetch = table(&[("https://a.test/x", BODY.to_vec())]);
        let result = load(
            &fetch,
            &layout(id(), &["https://a.test/x"]),
            id(),
            ObjectKind::Tree,
        )
        .await;
        assert_eq!(result.unwrap_err(), Error::Unheld);
    }
}
