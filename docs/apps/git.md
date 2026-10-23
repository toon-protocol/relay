# Git over TOON

A protocol guide for hosting and reading a git repository as Nostr events, with
the git objects on a Blossom server. It says **what goes in the packets** and in
what order. It names no client, CLI or library: anything that can sign Nostr
events, make HTTP requests and pay for writes can follow it.

Sources, in order of authority:

- [NIP-34](https://github.com/nostr-protocol/nips/blob/master/34.md) — the
  repository, patch, pull request, issue and status events.
- [`../nips/nip-34-blossom.md`](../nips/nip-34-blossom.md) — the `blossom`
  mapping tag, the object format, verification and expiry. A draft; its open
  questions are open here too.
- [ADR 0002](../adr/0002-readers-honour-repo-state-only-from-authorized-authors.md)
  — whose events a reader honours.
- [ADR 0001](../adr/0001-app-uis-live-on-the-relay.md) — the relay's `/repo` UI
  reads these events and the browser fetches objects itself.

Placeholders in `<angle brackets>` stand for values you compute. Other hashes
are placeholders too, except the object id `ce013625…` and blob id `2cf8d83d…`,
which are the ids of the git blob object `hello\n`.

## How writes work

Every event below is **written** by sending it to the relay's `POST /write`
through the connector, which enforces payment before the relay sees it; the
relay verifies the signature and stores the event. **Reads are free**: any
NIP-01 `REQ` over the relay's WebSocket. Blob uploads go to the Blossom
server, under that server's own payment rules. Payment is out of scope here.

## Terms

- **object id**: the 40-hex sha1 of a git object.
- **object bytes**: `<type> <size>\0<body>` (type is `blob`, `tree`, `commit`
  or `tag`; size is the decimal body length). The object id is the sha1 of
  these bytes.
- **blob id**: the 64-hex sha256 of the uploaded bytes.
- **owner**: the pubkey that signed the repository's `30617` announcement.
- **authorized authors**: the owner plus the pubkeys in the `maintainers` tag of
  the owner's own latest `30617` (ADR 0002).

The git object `blob 6\0hello\n` has object id
`ce013625030ba8dba906f756967f9e9ca394464a` and blob id
`2cf8d83d9ee29543b34a87727421fdecb7e3f3a183d337639025de576db9ebb4`.

## 1. Announce the repository (kind 30617)

A replaceable event, identified by `(pubkey, 30617, d)`. `d` is the repository
name and is a short, stable identifier.

```json
{
  "kind": 30617,
  "pubkey": "<owner pubkey>",
  "created_at": 1789999000,
  "tags": [
    ["d", "my-repo"],
    ["name", "my-repo"],
    ["description", "A small example repository"],
    ["web", "https://example.com/my-repo"],
    ["clone", "https://example.com/my-repo.git"],
    ["relays", "wss://relay.example.com"],
    ["r", "<earliest unique commit id>", "euc"],
    ["maintainers", "<maintainer pubkey>"],
    ["blossoms", "https://blossom.example.com", "https://blossom2.example.com"]
  ],
  "content": "",
  "id": "<event id>",
  "sig": "<signature>"
}
```

- `r` with `euc` is the root commit id (or earliest unique commit); other
  events use it to refer to the repository across forks.
- `maintainers` grants authority to move refs and set status (ADR 0002). Remove
  a pubkey from it to revoke that pubkey for all events, past and future.
- `blossoms` lists servers that may hold this repository's blobs. It names
  servers, not blobs, and is optional (see [Finding servers](#finding-servers)).
- A later `30617` replaces this one, so it has to carry every tag you still
  want.

## 2. Push

A push uploads objects first, then publishes the new state. Publishing the
state before the objects exist leaves readers pointing at nothing.

### 2.1 Choose the objects

Walk from the new tip commits and collect every commit, tree and blob object
the previous state did not already reach. For each, build the object bytes
(header included, **not** zlib-compressed) and compute the object id (sha1) and
blob id (sha256).

An object whose bytes exceed the Blossom server's size limit (about 1.9 MiB on a
TOON Blossom server) cannot be uploaded as one blob. Do not publish a
`blossom` tag for it. The NIP's fallback is an `arweave` tag; without one the
push is incomplete and you should not claim it complete.

### 2.2 Authorize each upload (BUD-11, kind 24242)

Each upload carries a signed authorization event naming the blob id in an `x`
tag. It is not published to a relay; it travels in an HTTP header.

```json
{
  "kind": 24242,
  "pubkey": "<author pubkey>",
  "created_at": 1790000000,
  "tags": [
    ["t", "upload"],
    ["x", "2cf8d83d9ee29543b34a87727421fdecb7e3f3a183d337639025de576db9ebb4"],
    ["expiration", "1790003600"]
  ],
  "content": "Upload object ce013625030ba8dba906f756967f9e9ca394464a",
  "id": "<event id>",
  "sig": "<signature>"
}
```

`created_at` is in the past, `expiration` in the future, and one event may
carry several `x` tags to authorize several blobs.

### 2.3 Upload (BUD-02)

Optionally check first: `HEAD /<blob id>` on the server (BUD-01). A `200`
means it already holds the blob, and you skip the upload (unless you are
renewing it, see [Extending retention](#expiry-and-retention)).

Otherwise:

```
PUT /upload HTTP/1.1
Host: blossom.example.com
Authorization: Nostr <base64url, unpadded, of the kind 24242 event JSON>
Content-Type: application/octet-stream
Content-Length: <length of the object bytes>
X-SHA-256: 2cf8d83d9ee29543b34a87727421fdecb7e3f3a183d337639025de576db9ebb4

<object bytes>
```

A success returns a blob descriptor:

```json
{
  "url": "https://blossom.example.com/2cf8d83d9ee29543b34a87727421fdecb7e3f3a183d337639025de576db9ebb4.bin",
  "sha256": "2cf8d83d9ee29543b34a87727421fdecb7e3f3a183d337639025de576db9ebb4",
  "size": 13,
  "type": "application/octet-stream",
  "uploaded": 1790000001
}
```

Check that `sha256` equals your blob id before going on. The server may require
payment for the upload; follow its rules.

### 2.4 Publish the state (kind 30618)

A replaceable event, `(pubkey, 30618, d)` with the same `d` as the announcement.
It holds the refs and the object mapping.

```json
{
  "kind": 30618,
  "pubkey": "<author pubkey>",
  "created_at": 1790000100,
  "tags": [
    ["d", "my-repo"],
    ["refs/heads/main", "9f3a1c6e0d2b4a7c8e5f1b3d6a9c0e2f4b7d8a1c"],
    ["HEAD", "ref: refs/heads/main"],
    ["blossom", "9f3a1c6e0d2b4a7c8e5f1b3d6a9c0e2f4b7d8a1c", "<blob id of that commit's object bytes>"],
    ["blossom", "ce013625030ba8dba906f756967f9e9ca394464a",
      "2cf8d83d9ee29543b34a87727421fdecb7e3f3a183d337639025de576db9ebb4"],
    ["arweave", "e1a4c8b2d6f09375a1c3e5b7d9f20486ac1e3b5d",
      "<arweave transaction id>"],
    ["blossom-index", "<blob id of the object index>"]
  ],
  "content": "",
  "id": "<event id>",
  "sig": "<signature>"
}
```

- One `["refs/...", <commit id>]` tag per branch and tag; `HEAD` names the
  default branch.
- One `["blossom", <object id>, <blob id>]` tag per object you want readers to
  find, at most one per object id.
- The event replaces the previous state, so **carry forward** the earlier
  `arweave` tags (never drop them) and the `blossom` tags for objects still
  wanted. The event has a size cap; it is a hint, not an index.
- For objects that do not fit in tags, upload an **object index** as a blob
  (lines of `<object id> <blob id>\n`, sorted by object id) and name it in
  `blossom-index`. The index is a draft proposal (NIP open questions 1–2).
- Only authorized authors' states count. A state from anyone else is ignored
  by readers, however new.

## 3. Fetch and clone

1. Read the owner's announcement: `["REQ", "a", {"kinds": [30617], "authors": ["<owner>"], "#d": ["my-repo"]}]`
   and take the newest. Its `maintainers` tag gives the authorized authors.
2. Read the state: `["REQ", "s", {"kinds": [30618], "authors": ["<owner>", "<maintainer>", ...], "#d": ["my-repo"]}]`.
   Take the newest by `created_at`; on a tie, the lowest event id (ADR 0002).
3. Resolve refs from that event's `refs/...` tags and `HEAD`.
4. Walk from the tip commit. For each object id you need, find its blob id in a
   `blossom` tag; if there is none, look in the `blossom-index` blob, then in
   an `arweave` tag; if none of those has it, the object is not retrievable
   (see [Expiry](#expiry-and-retention)).
5. For each blob id, try candidate servers (below) until one returns bytes that
   verify.

### Finding servers

In this order (a proposal, NIP open question 8):

1. the URLs in the announcement's `blossoms` tag;
2. the author's BUD-03 server list (kind `10063`);
3. any defaults the reader's operator configured.

Each candidate is `GET <server>/<blob id>`.

### Verify every object

1. Fetch over `https` only, refuse bodies over the size limit, apply a timeout.
2. The sha256 of the bytes must equal the blob id.
3. Parse the header; its `<size>` must equal the body length; the sha1 of the
   whole bytes must equal the object id.
4. On any failure, discard the bytes and try the next server.

A verified object never changes and can be cached indefinitely. Servers may
lie, so the checks are not optional. Objects stored by an `arweave` tag
follow that tag's format, which is not interchangeable with this one.

### Reading in a browser: the relay's `/repo` pages

A relay that serves `/repo` renders `/repo/<npub>/<d>/tree/<ref>/<path>` and
`/repo/<npub>/<d>/blob/<ref>/<path>`. `<ref>` is a branch or tag name from the
honoured state (it may hold slashes; the longest match wins) or a 40-hex commit
id. The relay **never fetches an object**: the page carries, as JSON in a
`data-git` attribute, the resolved commit and, for each object the honoured
`30618` names in an `arweave` or `blossom` tag, the URLs to try in order:

1. the `blossom` tag against the announcement's `blossoms` servers;
2. the `blossom` tag against the operator's `TOON_UI_BLOB_SERVERS`;
3. the `arweave` tag against the gateways **that tag names**:
   `["arweave", <object id>, <txId>, <gateway https URL>…]`. The gateways are
   optional extra values on the tag; a tag with none contributes no URL here;
4. the `arweave` tag against the operator's `TOON_UI_ARWEAVE_GATEWAYS`.

Blossom comes first and Arweave is the durable fallback, because a Blossom
blob may expire (see "Coexistence with Arweave tags" in
`docs/nips/nip-34-blossom.md`).

Each is `<gateway>/<txId>` or `<server>/<blob id>`; only `https` URLs are
listed. The page's script (a WebAssembly module) fetches over `https` only,
refuses an object over 2 MiB, times out each request, takes the first candidate
whose bytes hash to the object id, and renders the tree or file. Bytes may be
the object's body, the body behind its `<type> <size>\0` header, or the
zlib-compressed loose object. A missing, oversized or mismatched object shows
"this object is no longer held" in its place and the rest of the page still
renders. The browser's HTTP cache does the caching.

**Link previews.** A repository's home carries Open Graph and X card tags,
so a `/repo` link posted elsewhere unfurls into a card: `owner / name`, the
announcement's `description`, and a 1200×630 PNG the relay draws from its
own store (ADR 0006). While `/social` is on, the page names `/social`'s
image of the repo (`/social/og/r/<npub>/<d>.png`), so both apps share one
image counted as the social page counts; served alone, `/repo` names its own,
`/repo/og/<npub>/<d>.png` (stars, followers and the forks this relay holds).
Other `/repo` pages name `/repo/og/site.png`. The tags are absolute only when
the relay knows its public `https` origin (`TOON_UI_PUBLIC_URL`, else the
origin of a `wss://` `TOON_UI_RELAY_URL`); see "Link previews" in
[`social.md`](social.md) for the tags, the cache and its bounds.

## 4. Issues, patches, pull requests, comments and status

Issues, patches, pull requests and status events reference the repository by
an `a` tag, `30617:<owner pubkey>:<d>`, and tag the owner with `p` so they reach
the owner's notifications. Comments follow NIP-22 instead (below).

### Issue (kind 1621)

```json
{
  "kind": 1621,
  "pubkey": "<reporter pubkey>",
  "created_at": 1790001000,
  "tags": [
    ["a", "30617:<owner pubkey>:my-repo", "wss://relay.example.com"],
    ["p", "<owner pubkey>"],
    ["subject", "Crash when the config file is empty"],
    ["t", "bug"]
  ],
  "content": "Loading an empty config file panics.\n\nExpected: a clear error.",
  "id": "<event id>",
  "sig": "<signature>"
}
```

`content` is Markdown.

### Patch (kind 1617)

`content` is the output of `git format-patch` for a single patch. A series is a
chain: the first patch carries `t` `root`, and each later patch carries an `e`
tag with marker `reply` pointing at the previous patch.

```json
{
  "kind": 1617,
  "pubkey": "<contributor pubkey>",
  "created_at": 1790002000,
  "tags": [
    ["a", "30617:<owner pubkey>:my-repo", "wss://relay.example.com"],
    ["r", "<earliest unique commit id>"],
    ["p", "<owner pubkey>"],
    ["t", "root"],
    ["commit", "<commit id>"],
    ["parent-commit", "<parent commit id>"],
    ["commit-pgp-sig", ""],
    ["committer", "Ada Lovelace", "ada@example.com", "1790001900", "0"]
  ],
  "content": "From <commit id> Mon Sep 17 00:00:00 2001\nFrom: Ada Lovelace <ada@example.com>\nDate: Tue, 20 Oct 2026 10:00:00 +0000\nSubject: [PATCH] Handle an empty config\n\n---\n src/config.rs | 2 +-\n 1 file changed, 1 insertion(+), 1 deletion(-)\n\n...\n--\n2.45.0\n",
  "id": "<event id>",
  "sig": "<signature>"
}
```

A revision of a patch uses `t` `root-revision` instead of `root`, and its first
patch carries an `e` tag with marker `reply` pointing at the original root patch.

### Pull request (kind 1618)

For changes too large for a patch. The commits live on a clone URL or as
objects reachable in the Blossom store; the event names the tip.

```json
{
  "kind": 1618,
  "pubkey": "<contributor pubkey>",
  "created_at": 1790003000,
  "tags": [
    ["a", "30617:<owner pubkey>:my-repo", "wss://relay.example.com"],
    ["r", "<earliest unique commit id>"],
    ["p", "<owner pubkey>"],
    ["subject", "Rework config loading"],
    ["t", "refactor"],
    ["c", "<tip commit id>"],
    ["clone", "https://example.com/contributor/my-repo.git"],
    ["branch-name", "rework-config"],
    ["merge-base", "<merge base commit id>"]
  ],
  "content": "Moves config loading into its own module and adds tests.",
  "id": "<event id>",
  "sig": "<signature>"
}
```

### Pull request update (kind 1619)

Moves the tip of an existing pull request.

```json
{
  "kind": 1619,
  "pubkey": "<contributor pubkey>",
  "created_at": 1790003500,
  "tags": [
    ["a", "30617:<owner pubkey>:my-repo", "wss://relay.example.com"],
    ["r", "<earliest unique commit id>"],
    ["p", "<owner pubkey>"],
    ["P", "<PR author pubkey>"],
    ["E", "<kind 1618 event id>", "wss://relay.example.com"],
    ["c", "<new tip commit id>"],
    ["clone", "https://example.com/contributor/my-repo.git"],
    ["merge-base", "<merge base commit id>"]
  ],
  "content": "",
  "id": "<event id>",
  "sig": "<signature>"
}
```

### Comment (kind 1111, NIP-22)

Comments on an issue, patch or pull request. Uppercase tags name the root
(`E`, `K`, `P`); lowercase tags name the parent being replied to. For a
top-level comment the two are the same.

```json
{
  "kind": 1111,
  "pubkey": "<commenter pubkey>",
  "created_at": 1790004000,
  "tags": [
    ["E", "<kind 1621 event id>", "wss://relay.example.com", "<issue author pubkey>"],
    ["K", "1621"],
    ["P", "<issue author pubkey>", "wss://relay.example.com"],
    ["e", "<kind 1621 event id>", "wss://relay.example.com", "<issue author pubkey>"],
    ["k", "1621"],
    ["p", "<issue author pubkey>", "wss://relay.example.com"]
  ],
  "content": "I can reproduce this on 0.4.2.",
  "id": "<event id>",
  "sig": "<signature>"
}
```

To reply to another comment, set `e`/`k`/`p` to that comment (`k` is `1111`)
and leave `E`/`K`/`P` on the root.

### Status (kinds 1630–1633)

| Kind | Meaning                     |
| ---- | --------------------------- |
| 1630 | Open                        |
| 1631 | Applied / merged / resolved |
| 1632 | Closed                      |
| 1633 | Draft                       |

The newest status event by an honoured author is the current status. Honoured
authors are the authorized authors plus the author of the target issue, patch
or pull request (ADR 0002). Content is an optional Markdown note.

```json
{
  "kind": 1632,
  "pubkey": "<owner or maintainer pubkey>",
  "created_at": 1790005000,
  "tags": [
    ["e", "<kind 1621 event id>", "wss://relay.example.com", "root"],
    ["p", "<owner pubkey>"],
    ["p", "<issue author pubkey>"],
    ["a", "30617:<owner pubkey>:my-repo", "wss://relay.example.com"],
    ["r", "<earliest unique commit id>"]
  ],
  "content": "Closing: fixed in the next release.",
  "id": "<event id>",
  "sig": "<signature>"
}
```

Marking a patch applied (1631) adds, beside the root `e` tag, an `e` tag with
marker `reply` naming the accepted revision root (when a revision was applied),
a `q` tag for each applied patch event, and the commits that were applied, each
also as an `r` tag:

```json
{
  "kind": 1631,
  "pubkey": "<owner or maintainer pubkey>",
  "created_at": 1790006000,
  "tags": [
    ["e", "<kind 1617 root patch event id>", "wss://relay.example.com", "root"],
    ["e", "<kind 1617 accepted revision root event id>", "wss://relay.example.com", "reply"],
    ["p", "<owner pubkey>"],
    ["p", "<patch author pubkey>"],
    ["p", "<revision author pubkey>"],
    ["a", "30617:<owner pubkey>:my-repo", "wss://relay.example.com"],
    ["r", "<earliest unique commit id>"],
    ["q", "<kind 1617 applied patch event id>", "wss://relay.example.com", "<patch author pubkey>"],
    ["applied-as-commits", "<applied commit id>", "<applied commit id 2>"],
    ["r", "<applied commit id>"],
    ["r", "<applied commit id 2>"]
  ],
  "content": "Applied, thanks.",
  "id": "<event id>",
  "sig": "<signature>"
}
```

A merge instead of an apply carries `["merge-commit", <merge commit id>]` and
`["r", <merge commit id>]` in place of `applied-as-commits`. For a pull request
(1618), the root `e` tag names the 1618 event.

## Expiry and retention

A blob is kept for 30 days per payment and a server may drop it afterwards, with
no notice. Event data on the relay is not affected.

**When objects have expired** (a `404`, a timeout, an over-limit response, or
bytes that fail verification):

- Treat it as "this object is no longer held", not as corruption of the
  repository. Show everything else that is readable.
- Try the next candidate server, then any `arweave` tag for the same object.
- If none works, the object is lost to readers until a writer republishes it.

**Extending retention** is a writer's job and needs no new event:

- Pay for the same blob again by re-uploading identical bytes (section 2.3,
  same blob id), or ask a server to mirror it (BUD-04), whichever that server's
  payment rules treat as a renewal. Whether it extends the remaining time or
  restarts the 30 days, and whether it renews at all on a TOON Blossom server,
  are open questions in the NIP (question 6); check the server.
- The `30618` tags do not change because the blob ids do not.
- Renew every blob the state names, including the object index, at an interval
  shorter than 30 days.
- The event cannot tell a reader how long a blob has left; readers must not
  infer one.

To restore objects that already expired, a writer who still has them repeats
sections 2.2 and 2.3 for those blobs. Nothing is republished to the relay
unless the mapping itself has to change.
