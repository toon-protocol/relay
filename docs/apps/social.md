# Social over TOON, repos first

A protocol guide for the social side of building in public on Nostr: notes,
launches, stars, follows and talk about repositories. It says **what goes in
the events** a client publishes and how the relay's `/social` pages read them.
It names no client, CLI or library: anything that can sign Nostr events and
pay for writes can follow it.

Sources, in order of authority:

- The NIPs each section names (NIP-01, -02, -10, -18, -22, -23, -25, -27,
  -51, -5A, -68, -71, -84, -89, -92, -C0, -7D), and NIP-34 for forks. NIP-5A,
  NIP-89 and NIP-34's fork tags were read at nostr-protocol/nips@d47a2ec.
- [`git.md`](git.md) — repositories themselves (NIP-34) and whose repo state
  a reader honours.
- [ADR 0001](../adr/0001-app-uis-live-on-the-relay.md) — the relay's UIs read
  only this relay's store, write nothing and fetch nothing.
- [ADR 0002](../adr/0002-readers-honour-repo-state-only-from-authorized-authors.md)
  — authorized authors: the owner plus the owner's `maintainers`.
- [`../social-for-builders-ux.md`](../social-for-builders-ux.md) — the research
  note the conventions below come from (§7.0 and §8).

Placeholders in `<angle brackets>` stand for values you compute. `A` means a
repository's address, `30617:<owner pubkey>:<d>`.

## How writes work

Every event below is **written** by sending it to the relay's `POST /write`
through the connector, which enforces payment; each star, follow, comment or
launch is one paid write. **Reads are free**. The `/social` pages are a reader
like any other: they show what was written and never write (ADR 0001). The
connector states the payer and amount on each delivery but the relay does not
store it, so a page can say "N people starred this, each a paid write", never
how much anyone paid.

## The three rules

1. **A repo is its address `A`, never a URL.** Every social act on a repo
   names `A` in an `a` tag (or, for a NIP-22 comment, an `A` root tag).
2. **The maker is kept apart from the crowd.** An event about `A` by an
   authorized author (ADR 0002) is the maker's voice; anyone else's is the
   community's. Pages show them apart.
3. **Count people, not events.** Stars, followers, talk and trending count
   distinct pubkeys. A NIP-09 deletion (kind 5) removes an event from the
   store, so it also leaves every count.

## Conventions

| Act | Event | Required tags | Notes |
| --- | --- | --- | --- |
| Star a repo | kind 7, content `+` | `a` = `A`, `k` = `30617` | NIP-25 on an addressable event. `-`, an emoji or a missing `k` is not a star. Un-star with a kind 5 naming the 7 |
| Follow a repo | kind 10018 | one `a` per followed `A` | NIP-51 "git repositories". The list is replaced whole on each change |
| Follow code authors | kind 10017 | one `p` per key | NIP-51 "git authors"; read beside the kind 3 follow list |
| Boost a repo | kind 16 | `a` = `A`, `k` = `30617` | NIP-18 generic repost; content may be the announcement's JSON |
| Mention a repo | any text | `nostr:naddr1…` of kind 30617 in the content | NIP-27; add an `a` tag too so the repo page lists it |
| Talk about a repo | kind 1, 11 or 9802 | `a` = `A` | A note, a NIP-7D thread or a NIP-84 highlight |
| Comment on a repo | kind 1111 | `A` = `A`, `K` = `30617` (and `a`/`k` for the parent) | NIP-22 |
| Ship a repo | kind 1 by an authorized author | `a` = `A`, and `t` = `ship` or `imeta` media | The convention this guide adds; see below |
| Show it | kind 20, 21 or 22 | `a` = `A`, `imeta` | NIP-68 pictures, NIP-71 videos |
| Share code | kind 1337 | `a` = `A`, or `repo` = `A`, its `naddr` or its `nostr://` URL | NIP-C0 snippet |
| Write it up | kind 30023 | `a` = `A` | NIP-23 article; by the maker it is a devlog |

### A ship post

A launch is a note by the repo's owner or a maintainer that names the repo
and is marked as a launch:

```json
{
  "kind": 1,
  "pubkey": "<owner or maintainer pubkey>",
  "created_at": 1790010000,
  "tags": [
    ["a", "30617:<owner pubkey>:tidepool", "wss://relay.example.com"],
    ["t", "ship"],
    ["imeta", "url https://blossom.example.com/<sha256>.mp4", "m video/mp4", "image https://blossom.example.com/<poster sha256>.jpg", "alt A 40-second demo"],
    ["imeta", "url https://blossom.example.com/<sha256>.png", "m image/png", "dim 1200x675", "alt The 7-day chart"]
  ],
  "content": "Shipped tidepool: tide charts for small-boat sailors, offline-first.\n\nhttps://blossom.example.com/<sha256>.png",
  "id": "<event id>",
  "sig": "<signature>"
}
```

- Either the `t` `ship` tag or at least one `imeta` media tag marks it; a note
  by anyone else is talk, however it is tagged.
- `imeta` (NIP-92) describes media whose URL is in the content. The first
  becomes the launch's hero; only `https` images and videos are drawn.
- The headline is the content's first sentence.

A Shakespeare app event is a launch too: a kind 31990 by an authorized author
that tags the repo **and** the NIP-5A site it deployed:

```json
{
  "kind": 31990,
  "tags": [
    ["d", "surfcast-app"],
    ["a", "30617:<owner pubkey>:surfcast"],
    ["a", "35128:<owner pubkey>:surfcast"],
    ["t", "shakespeare"]
  ],
  "content": "{\"name\":\"surfcast\",\"about\":\"Surf forecasts for your break.\"}"
}
```

### A live demo

A repo's live demo is the maker's to name: only an event signed by the owner
or a maintainer counts. Every page (the repo card, the repo page's header and
About, a launch) uses the same rule, first match wins:

1. a launch that is a Shakespeare app event (kind 31990) names its site in an
   `a` tag, and an authorized author signed that site;
2. else the newest NIP-5A site manifest (kind 35128, or the root site 15128)
   with at least one `path` tag whose `source` tag is a `nostr://` clone URL
   of the repo in any of NIP-34's forms: `nostr://<naddr>`,
   `nostr://<npub>/[<relay>/]<d>`, or the owner's NIP-05 name in place of
   the npub (`domain` alone meaning `_@domain`);
3. else the newest app event that `a`-tags the repo: a kind 31990 naming a
   site, as in 1, or a kind 32267 software application with an `https`
   `url` tag. Kind 32267 is the "NIP-82" draft, **nips#1336, an open PR,
   not a merged NIP**.

```json
{
  "kind": 35128,
  "tags": [
    ["d", "tidepool"],
    ["path", "/index.html", "<sha256>"],
    ["x", "<aggregate sha256>", "aggregate"],
    ["source", "nostr://<owner npub>/relay.example.com/tidepool"]
  ]
}
```

The relay serves nothing of the site: `/social` links it through the
operator's gateway (`TOON_UI_NSITE_GATEWAY`) in NIP-5A's canonical forms,
`https://<pubkey in base36><d>.<gateway host>/` for a named site (the base36
key is always the 50 characters the NIP names; `d` must match
`^[a-z0-9-]{1,13}$` and not end in `-`, or there is no link) and
`https://<npub>.<gateway host>/` for a root site. With no gateway set, a
site is no demo; a kind 32267's `url` is linked as it is either way.

### Forks and remixes

A fork or a remix is written by whoever made the copy, in their own events.
Nothing about it is the original maker's to approve, and nothing is a write
by this page. **Forks and remixes are public and permanent**: a NIP-09
deletion request removes one from this relay, but other relays may keep it.

**Fork a repo (NIP-34).** Announce your copy as your own kind 30617 with a
`u` tag naming the repo you forked, and the git URL you cloned it from after
a `|`. Keep the parent's earliest unique commit in an `r … euc` tag unless
your fork has diverged for good (then NIP-34 says to use the first commit
after the fork):

```json
{
  "kind": 30617,
  "tags": [
    ["d", "tidepool"],
    ["name", "tidepool"],
    ["description", "Fixes for New Zealand stations."],
    ["u", "30617:<parent owner pubkey>:tidepool|nostr://<parent npub>/relay.example.com/tidepool", "wss://relay.example.com", "<parent owner pubkey>"],
    ["r", "<earliest unique commit>", "euc"]
  ]
}
```

A repo that has the same `euc` commit and no `u` tag is still shown, apart,
as "same first commit": the same project announced again, or a fork that did
not say so.

**Remix a site (NIP-5A).** Copy the site's manifest (`path` tags, and
`title`, `description`, `source` and `app` where they still apply), name the
site you copied in exactly one `a` tag and the origin of the lineage in
exactly one `A` tag. Copying straight from the original, both name it;
copying a copy, `A` is the parent's `A`, unchanged. Point `source` at your
own repo if your remix has one, so the remix links to its code:

```json
{
  "kind": 35128,
  "tags": [
    ["d", "surfcast"],
    ["a", "35128:<parent site pubkey>:tidepool"],
    ["A", "35128:<origin site pubkey>:tidepool"],
    ["path", "/index.html", "<sha256>"],
    ["x", "<aggregate sha256>", "aggregate"],
    ["source", "nostr://<your npub>/relay.example.com/surfcast"]
  ]
}
```

A root site's address is `15128:<pubkey>:` (NIP-01's form for a replaceable
event). A copy whose files hash to its parent's NIP-5A aggregate is shown as
an **identical copy**. The page computes the aggregate itself from the
`path` tags (SHA-256 of the sorted `<sha256> <path>\n` lines); the `x` tag a
copier wrote is only how a lookalike is found. A site with the repo's own
aggregate, another author and no `a` or `A` tag is shown as **same site**,
an uncredited copy.

### Open in… (NIP-89)

An app that can show a repository announces a kind 31990 handler that
declares `k` `30617` and a `web` URL template for an `naddr`; anyone who uses
it recommends it with a kind 31989 whose `d` is `30617`:

```json
{ "kind": 31990, "content": "{\"name\":\"gitworkshop\"}",
  "tags": [["d", "repos"], ["k", "30617"], ["web", "https://gitworkshop.example/<bech32>", "naddr"]] }
{ "kind": 31989,
  "tags": [["d", "30617"], ["a", "31990:<app pubkey>:repos", "wss://relay.example.com", "web"]] }
```

The repo page lists only handlers someone recommends (NIP-89 warns against
trusting bare 31990s), ranked by the distinct people who recommend them,
each person's newest 31989 only. With `?as=<npub>` the viewer and the people
the viewer follows (kind 3 and kind 10017 `p` tags) count first. The page
uses the `web` tag marked `naddr`, else the generic one with no entity type,
only over `https`, and fills NIP-89's `<bech32>` placeholder (`<nip19>` is
read the same way) with the repo's `naddr`, with this relay as its hint.

**The Remix rule.** NIP-89 has no way for a handler to say it can clone a
repository, so a **Remix** link is drawn only for a recommended handler
whose `web` template is on a host the relay's code knows clones `nostr://`
repos, and nothing else. Today that list is one entry: `shakespeare.diy`,
whose `/clone?url=` page takes a `nostr://` URI; the link is
`https://shakespeare.diy/clone?url=<percent-encoded nostr:// clone URL>`.
Adding a builder is a code change citing where that builder accepts
`nostr://` (`crates/ui-social/src/handlers.rs`, `CLONERS`).

## Reading in a browser: the relay's `/social` pages

| Path | What it shows | What it reads |
| --- | --- | --- |
| `/social` | Everyone here: notes that start a conversation, repo boosts, launches, articles, merges, new repos; `?until=<unix>` pages back | kinds 1, 16, 30023, 31990, 1631, 30617 |
| `/social/following/<npub>` | What that key follows; the top bar's form gets here | its kind 3 and 10017 `p` tags (their posts), its 10018 `a` tags (posts, issues and pull requests on those repos) |
| `/social/r/<npub>/<d>` | The repo as a profile; tabs `/talk`, `/stars`, `/followers`, `/forks`, `/media`, `/snippets`; `?as=<npub>` ranks "Open in…" by that key's follows | `{"#a":[A]}`, `{"kinds":[1111],"#A":[A]}`, kind 7 and 10018 by `#a`, recent 1337s for a `repo` tag, kinds 31989 (`#d` 30617) and 31990 (`#k` 30617) |
| `/social/r/<npub>/<d>/forks` | Forks and remixes as a tree, and repos with the same first commit | up to 2,000 kind 30617s (the `u` value holds a URL, so it is scanned, not indexed) plus `#r` the `euc` commit; the repo's sites' copies by `#A`/`#a`, lookalikes by `#x` |
| `/social/r/<naddr>` | A redirect to the repo's page | — |
| `/social/e/<id>` | One post (hex, `note` or `nevent`) with its replies; a launch when it is one | the event, kind 1 replies by `e`, kind 1111 by `E`, kind 7 reactions by `e` |

A note's replies are on its page, not in the feed. A merge (kind 1631) shows
only from an authorized author or the change's own author (ADR 0002, rule 3).

**The repo card.** Wherever a post names a repo, by an `a` tag or a
`nostr:naddr` in its text, it unfurls into a card: name, description, owner,
`t` topics, stars, followers, open issues and pull requests (by honoured
status), when an authorized author last moved a ref, Open repo (`/repo`) and a
clone line. The clone line is `nostr://<npub>/<relay host>/<d>`, the host
being `TOON_UI_RELAY_URL`'s (else the paid feed's `TOON_RELAY_URL`); when the
owner's kind 0 claims a NIP-05 name, a second line uses it, marked unverified,
because the relay never fetches the `.well-known` document that would prove it.

**The repo page's tabs.** Maker lists posts about the repo by authorized
authors (notes, threads, comments, highlights, articles, app events); Talk
lists everyone else's notes, threads, comments and highlights, and counts the
people; Stars and Followers list each key once, newest first; Forks draws the
lineage (below); Media lists pictures and videos; Snippets lists code
snippets. Code goes back to `/repo`,
and `/repo`'s repository header has a Social tab back here.

**The Forks tab.** Nested lists, drawn on the server: the chain of parents up
to the origin (at most four), each parent's other forks, this repo marked,
and its forks three levels down (at most 25 per repo, then "and N more").
The remixes of this repo's sites hang under it, each with a Try it link
through the gateway and a link to the code its `source` names. Repos with
the same first commit and no `u` tag are listed apart. The tab's count is
people: the distinct keys that forked or copied the repo or its sites, its
makers left out. The repo header (here and on `/repo`) and the repo card say
"Forked from owner / name". The sidebar adds the lineage at a glance, the
Remix card (the Remix rule above, then how to fork with git) and the public
and permanent note.

**Trending today.** The repos the most distinct keys named (in an `a` or `A`
tag of a note, reaction, boost, comment, issue, pull request or article) on
this relay in the last 24 hours. People, not events, and only this relay's.

### What a page loads

`/social` pages run no script. Their policy is

```
default-src 'self'; script-src 'none'; img-src 'self' https:; media-src 'self' https:; form-action 'self'; base-uri 'none'
```

so the visitor's **browser** loads the images and videos a post names, from
wherever the post says, over `https` only. The relay itself never fetches them
(ADR 0001). Pages send `Referrer-Policy: no-referrer` (and each `<img>` says
so too), so a media host is not told which post was read, and a video is not
requested until it is played (`preload="none"`). Avatars are initials, never a
profile's `picture`, so reading a feed does not load a picture per person.
`/repo`'s pages keep their stricter policy.

## Trying it locally

A dev seed loads a small, realistic community (profiles, two repos and a
Shakespeare app, stars, follows, a ship post with screenshots and a live demo,
an `naddr` mention, a boost, comments, a thread, a highlight, an article, a
picture, a snippet, an issue and a merge) into a relay on this machine:

```bash
cargo build -p relay
TOON_SECRET_KEY=$(printf '1%.0s' {1..64}) TOON_DATA_DIR=/tmp/social-dev \
  TOON_UI_RELAY_URL=ws://127.0.0.1:7100 TOON_UI_NSITE_GATEWAY=https://nsite.lol \
  target/debug/relay
# in another shell, from the repo root:
pnpm install
WRITE_URL=http://127.0.0.1:3100 READ_URL=http://127.0.0.1:7100 \
  pnpm exec tsx packages/conformance/seed/social.ts
```

The script prints the pages to open. A relay started without a connector
accepts `POST /write` directly, which is how the seed writes. The fixture is
`packages/conformance/suite/harness/social-fixture.ts`, with forks, remixes,
handlers and an app listing added by `lineage-fixture.ts`; the conformance cases
in `packages/conformance/suite/ui-social.test.ts` and `ui-social-lineage.test.ts`
seed the same events. Its
media are placeholder images from picsum.photos, which your browser loads.
