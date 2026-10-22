NIP-34 extension: git objects on Blossom
========================================

`draft` `optional`

This draft extends [NIP-34](https://github.com/nostr-protocol/nips/blob/master/34.md)
so that the git objects of a repository are stored on [Blossom](https://github.com/hzrd149/blossom)
servers instead of Arweave. It adds one tag to the repository state event
(kind `30618`) and says how a reader finds, checks and refreshes the objects
it names. It changes no other NIP-34 event.

It is a draft for the maintainer. Choices it cannot settle are listed under
[Open questions](#open-questions), and the text does not guess at them.
Publishing it through the NIP process is out of scope here.

## Background

Today `rig` stores one loose git object per Arweave upload and maps the git
object id (sha1) to the Arweave transaction id with tags on the `30618` event:

```
["arweave", <sha1>, <txId>]
```

Blossom differs from Arweave in three ways that shape this draft:

- **Content addressing.** A blob is addressed by the sha256 of its bytes
  (BUD-01), not by git's sha1. The two ids have to be mapped.
- **Size.** A TOON Blossom server (`toon-protocol/blossom`) cannot hold a blob
  over about 1.9 MiB. Call this the *blob limit*. Servers MAY advertise a
  different limit; clients MUST NOT assume one larger than 1.9 MiB.
- **Retention.** A blob is kept for 30 days per payment, then may be dropped.
  Arweave storage is permanent.

## Terms

- **object id**: the 40-hex sha1 of a git object, as git computes it.
- **object bytes**: `<type> <size>\0<body>`, where `<type>` is `blob`, `tree`,
  `commit` or `tag`, `<size>` is the decimal byte length of `<body>`, and
  `<body>` is the object's content. The object id is the sha1 of exactly these
  bytes. This is git's loose object before zlib compression.
- **blob id**: the 64-hex sha256 of the uploaded bytes.

## Mapping tag

A `30618` event MAY carry, for each object it wants readers to find, a tag:

```
["blossom", <object id>, <blob id>]
```

- `<object id>` is 40 lowercase hex characters, `<blob id>` is 64.
- The tag maps an object to the blob that holds its object bytes. It does not
  say which server holds it (see [Finding servers](#finding-servers)).
- There is at most one `blossom` tag per object id in an event. Readers that
  see several use the first.
- A `blossom` tag is a claim by the author. A reader MUST check it (see
  [Verification](#verification)) and MUST ignore the bytes if they do not match.
- Only refs from authorized authors count, as in NIP-34 and the existing
  `arweave` handling. The tag adds no new authority.

### Objects not listed

The `30618` event is replaceable and relays and clients cap event size, so it
cannot list every object of a large repository. The tag list is therefore a
**hint, not an index**. A reader that needs an object with no `blossom` tag:

A reader can learn an unlisted object's id by walking from a listed object
(a commit names its tree, a tree names its entries), but it cannot compute the
blob id from the object id. An unlisted object is therefore **not retrievable**
unless the blob id is published somewhere.

To make unlisted objects retrievable, a writer MUST publish an **object index**
as a blob. The index is the bytes of lines `<object id> <blob id>\n`, sorted by
object id, and its own blob id is named in a tag on the `30618` event:

```
["blossom-index", <blob id>]
```

A reader that finds no `blossom` tag for an object and has a `blossom-index`
tag fetches and verifies the index and reads the pair from it. The index is
itself a blob and obeys the blob limit; a line is 106 bytes, so one index holds
about 18,000 objects. Larger repositories need more than one index (open
question 2). Tags in the event win over the index when both name an object.

## Loose objects, not packs

Each git object is uploaded as its own blob, as `rig` does on Arweave:

- Readers (including browsers) can fetch and check exactly the object they
  need, with no pack index and no delta resolution.
- An object is immutable and addressed by its content, so a blob can be cached
  forever and shared by every repository that holds the same object.
- A push uploads only objects the server does not already hold (see
  [Upload](#upload)).

Packs are not used in this version. They would resolve the many-small-blobs
cost and most of the size problem, at the price of readers needing delta and
index logic. Open question 3 records the decision for the maintainer.

## What is uploaded

The **object bytes**, with the header, uncompressed. Reasons:

- The sha1 of the downloaded bytes is the object id, so a reader checks it
  without knowing the object's type in advance.
- The type and size are in the bytes, so a reader can reject an object of an
  unexpected type before parsing it.
- Blossom serves the bytes with `Content-Type: application/octet-stream`
  unless the uploader says otherwise; nothing depends on it.

The header is not stripped and the body is not zlib-compressed. Compression
is left to the transport (`Content-Encoding`) and the server. If `rig`'s
current Arweave payload differs (header or compression), readers of `arweave`
tags keep handling that format; the two formats are not interchangeable
(open question 4).

### Upload

A writer uploads with BUD-02 `PUT /upload`, with a BUD-11 authorization event
naming the blob id. Before uploading, it MAY `HEAD /<blob id>` (BUD-01) and
skip the upload if the server already holds the blob. Payment for the upload
is whatever the server's gateway requires and is out of scope here.

## Objects over the size limit

An object whose object bytes exceed the blob limit **cannot be stored as one
blob**. This version does not split objects. A writer with such an object:

- MUST NOT publish a `blossom` tag for it;
- SHOULD store it by another means a reader understands, today the `arweave`
  tag (`["arweave", <object id>, <txId>]`), and publish that tag instead;
- if it has no such means, the repository's object set is incomplete, and the
  writer SHOULD NOT claim the push complete.

A reader meeting an object with no usable tag and no index entry treats it as
*not held* (see [Expiry](#expiry-and-retention)), not as corruption.

Splitting an object into chunks needs a new manifest format and is not
specified here (open question 5).

## Expiry and retention

A blob lives for 30 days per payment and a server MAY drop it afterwards.
There is no notification. Consequences, stated so readers and writers behave
the same way:

- **Readers.** A `404`, a timeout, an over-limit response or a body whose
  sha1 does not match is a normal state, "this object is no longer held". A
  reader MUST NOT treat it as an error of the whole repository and SHOULD
  still show everything else it can. It tries the next server and then any
  `arweave` tag for the same object (see [Coexistence](#coexistence-with-arweave-tags)).
- **Writers extend retention** by paying for the same blob again, that is by
  re-uploading identical bytes (same blob id) or by asking a server to mirror
  it (BUD-04), whichever the server's payment rules treat as a renewal. The
  tag in the `30618` event does not change, because the blob id does not.
  Whether a renewal extends the *remaining* time or restarts the 30 days is a
  server behaviour this draft does not state (open question 6).
- A writer that wants a repository to stay readable SHOULD renew every blob
  it names, including the object index, at an interval shorter than 30 days.
  A renewal run does not need a new `30618` event.
- Because a renewal does not touch the event, the event cannot tell a reader
  how long a blob has left. Readers MUST NOT infer one.

## Finding servers

The tag names a blob, not a server. A reader builds its list of candidate
servers, in this order, and tries them until one returns bytes that verify:

1. the URLs in the repository announcement's `blossoms` tag (below);
2. the Blossom servers in the author's BUD-03 server list (kind `10063`);
3. any servers the reader's operator configured as defaults.

Each candidate is `<server>/<blob id>` (BUD-01). A reader MAY append a file
extension; it MUST NOT depend on one.

### The `blossoms` announcement tag

ngit puts a tag on the `30617` announcement:

```
["blossoms", <server url>, <server url>, ...]
```

Under this draft it keeps its meaning: *these Blossom servers may hold this
repository's blobs*. It is a list of servers, not of blobs, so it needs no
change. The tag is optional, so a repository that sets it names where
objects are; one that does not still works through steps 2 and 3.

## Verification

A reader that fetches `<server>/<blob id>`:

1. MUST fetch over `https` only, refuse a response over the blob limit, and
   apply a timeout;
2. MUST check that the sha256 of the bytes is the blob id (servers MAY lie);
3. MUST parse the header, check that the `<size>` equals the body length, and
   check that the sha1 of the whole bytes equals the object id;
4. MUST discard the bytes if any check fails, and try the next server.

An object that has passed these checks never changes, so it MAY be cached
without expiry.

## Coexistence with Arweave tags

- A `30618` event MAY carry both `arweave` and `blossom` tags, even for the
  same object id. Readers that do not know `blossom` ignore it.
- For an object with both, a reader tries Blossom first, because it is
  cheaper to fetch, and falls back to Arweave when the blob has expired or
  is over the limit. The Arweave copy is the durable one.
- A writer MAY stop uploading to Arweave for new objects. It MUST NOT remove
  `arweave` tags it published earlier: older refs still depend on them.
- Existing repositories need no migration. The first push after a writer
  adopts this draft adds `blossom` tags for the objects it uploads, and the
  event it replaces keeps its `arweave` tags.

## Examples

A repository state event, with refs, one object on Blossom, one only on
Arweave because it is over the blob limit, and an index:

```json
{
  "kind": 30618,
  "pubkey": "<author pubkey>",
  "created_at": 1790000000,
  "tags": [
    ["d", "my-repo"],
    ["refs/heads/main", "9f3a1c6e0d2b4a7c8e5f1b3d6a9c0e2f4b7d8a1c"],
    ["HEAD", "ref: refs/heads/main"],
    ["blossom", "9f3a1c6e0d2b4a7c8e5f1b3d6a9c0e2f4b7d8a1c",
      "b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9"],
    ["blossom", "2c7b0e4d9a1f3c5e8b6a0d2f4c7e9b1a3d5f8c0e",
      "7f83b1657ff1fc53b92dc18148a1d65dfc2d4b1fa3d677284addd200126d9069"],
    ["arweave", "e1a4c8b2d6f09375a1c3e5b7d9f20486ac1e3b5d",
      "Zk3q8vWm0N1xY5tA7rB9cD2eF4gH6iJ8kL0mN2oP4qR"],
    ["blossom-index",
      "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"]
  ],
  "content": "",
  "id": "<event id>",
  "sig": "<signature>"
}
```

The announcement naming the servers (ngit's existing tag, unchanged):

```json
{
  "kind": 30617,
  "pubkey": "<author pubkey>",
  "created_at": 1789999000,
  "tags": [
    ["d", "my-repo"],
    ["name", "my-repo"],
    ["clone", "https://example.com/my-repo.git"],
    ["blossoms", "https://blossom.example.com", "https://blossom2.example.com"]
  ],
  "content": "",
  "id": "<event id>",
  "sig": "<signature>"
}
```

A blob `7f83b165…` is the bytes of a git object, for example a blob object
with the body `hello\n`:

```
blob 6\0hello\n
```

Its sha1 is `ce013625030ba8dba906f756967f9e9ca394464a` (the object id) and
its sha256 is the blob id the tag maps it to. A reader that fetches
`https://blossom.example.com/<blob id>` checks both.

The object index blob, one pair per line:

```
2c7b0e4d9a1f3c5e8b6a0d2f4c7e9b1a3d5f8c0e 7f83b1657ff1fc53b92dc18148a1d65dfc2d4b1fa3d677284addd200126d9069
9f3a1c6e0d2b4a7c8e5f1b3d6a9c0e2f4b7d8a1c b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9
```

Hashes in the examples are placeholders, except the sha1 of `blob 6\0hello\n`.

## Open questions

For the maintainer. None is settled by this draft.

1. **Index tag vs. tags only.** Is the `blossom-index` tag wanted at all, or
   should a writer always list every object it uploads in tags and accept the
   event-size cap? The draft proposes the index because tags alone cannot
   reach unlisted objects.
2. **Large repositories.** One index holds about 18,000 objects. Do larger
   repositories chain indexes (an index naming the next), shard by object id
   prefix, or use packs?
3. **Loose objects or packs.** The draft picks loose objects for the
   one-fetch-per-object read path and for sharing. Is that the final choice,
   and if packs are added, what is the tag (`["blossom-pack", <sha256>]`?) and
   how does a reader find an object inside a pack?
4. **Payload format against `rig`.** Does `rig` upload to Arweave with the
   header, and compressed? If it differs from "header, uncompressed", does
   the NIP prefer one format for both, or keep two? The draft states the
   Blossom format only.
5. **Objects over the blob limit.** The draft sends them to Arweave. Should a
   later version define a chunk manifest (a blob listing the sha256 of each
   chunk, with a tag such as `["blossom-chunks", <object id>, <manifest
   blob id>]`), or is "Arweave only" acceptable?
6. **Renewal semantics.** Does re-uploading or mirroring an existing blob
   extend its remaining 30 days or restart them, and does the server charge
   the full price either way? This is a `toon-protocol/blossom` behaviour the
   NIP should cite once it is fixed.
7. **Renewal discovery.** Should the repository announce when it last renewed
   (a tag with a timestamp) so readers can estimate what is still held? The
   draft says readers must not infer a time and offers no tag.
8. **Server list precedence.** Should kind `10063` (BUD-03) servers be tried
   before or after the `blossoms` tag, and is `blossoms` a statement by the
   repository or only by its first author?
9. **Maintainer vs. author.** `30618` is signed by one author. Where several
   authors or maintainers publish state, whose `blossom` tags and index does a
   reader use, and does the authorized-authors rule extend to the index blob?
10. **Deletion.** Should a writer be able to say "I deleted this blob" (BUD-02
    `DELETE`), and does a reader's cache hold after that? Content addressing
    suggests no further action, but it is unstated.
