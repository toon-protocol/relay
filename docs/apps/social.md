# Social over TOON, repos first

A protocol guide for the social side of building in public on Nostr: notes,
launches, stars, follows and talk about repositories. It says **what goes in
the events** a client publishes and how the relay's `/social` pages read them.
It names no client, CLI or library: anything that can sign Nostr events and
pay for writes can follow it.

Sources, in order of authority:

- The NIPs each section names (NIP-01, -02, -10, -18, -22, -23, -25, -27,
  -51, -5A, -68, -71, -84, -89, -92, -C0, -7D).
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
| Keep a series | kind 30004 by the articles' author | `d`, `title`, one `a` = `30023:<author>:<d>` per part, in order | NIP-51 curation set; see [Devlogs](#devlogs-series-highlights-and-snippets) |
| Highlight an article | kind 9802 | `a` = the article's `30023:<author>:<d>` (and `p` = author) | NIP-84; content is the passage, word for word |
| Comment on an article | kind 1111 | `A` = the article's address, `K` = `30023`, `P` = author | NIP-22; a kind 7 or 16 on it carries `a` (and `e`) and `k` = `30023` |

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

A repo's live demo is a NIP-5A site manifest (kind 35128, or the root site
15128) signed by an authorized author whose `source` tag is the repo's
`nostr://` URL, `nostr://<owner npub>/<relay host>/<d>` (the relay hint is
optional; the owner's NIP-05 name may stand for the npub). A Shakespeare app
event names its site in its `a` tag instead. The relay serves nothing of the
site: `/social` links it through the operator's gateway
(`TOON_UI_NSITE_GATEWAY`) as `https://<pubkey in base36><d>.<gateway host>/`
(a root site: `https://<npub>.<gateway host>/`), and shows no demo when no
gateway is set.

## Reading in a browser: the relay's `/social` pages

| Path | What it shows | What it reads |
| --- | --- | --- |
| `/social` | Everyone here: notes that start a conversation, repo boosts, launches, articles, merges, new repos; `?until=<unix>` pages back | kinds 1, 16, 30023, 31990, 1631, 30617 |
| `/social/following/<npub>` | What that key follows; the top bar's form gets here | its kind 3 and 10017 `p` tags (their posts), its 10018 `a` tags (posts, issues and pull requests on those repos) |
| `/social/r/<npub>/<d>` | The repo as a profile; tabs `/talk`, `/stars`, `/followers`, `/media`, `/snippets` | `{"#a":[A]}`, `{"kinds":[1111],"#A":[A]}`, kind 7 and 10018 by `#a`, recent 1337s for a `repo` tag |
| `/social/r/<naddr>` | A redirect to the repo's page | — |
| `/social/e/<id>` | One post (hex, `note` or `nevent`) with its replies; a launch when it is one. An article or a snippet redirects to its own page | the event, kind 1 replies by `e`, kind 1111 by `E`, kind 7 reactions by `e` |
| `/social/a/<naddr>` | An article (also `/social/a/<npub>/<d>`): its series, highlights, reactions and comments | the newest 30023 at the address; 30004 by its author with its `a`; 9802 and 7 by `#a` or `#e`; 16 by `#a`; 1111 by `#A` |
| `/social/s/<id>` | A code snippet (hex, `note` or `nevent`) with its repo, reactions and replies | the 1337; kind 7 by `e`; replies as a post's |
| `/social/r/<npub>/<d>/writing` | The repo's Writing tab: every article that tags it, the maker's marked | kind 30023 by `#a` |

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
people; Stars and Followers list each key once, newest first; Media lists
pictures and videos; Snippets lists code snippets; Writing lists every
article that tags the repo, the maker's marked. Code goes back to `/repo`,
and `/repo`'s repository header has a Social tab back here.

**Trending today.** The repos the most distinct keys named (in an `a` or `A`
tag of a note, reaction, boost, comment, issue, pull request or article) on
this relay in the last 24 hours. People, not events, and only this relay's.

### Devlogs, series, highlights and snippets

NIP-23, NIP-51, NIP-84 and NIP-C0 are all **drafts** upstream; what follows
is how these pages read them today.

**An article** (kind 30023) that `a`-tags a repo is a devlog when its author
is an authorized author of the repo, and a write-up when anyone else wrote
it; both are on the repo's Writing tab, and the maker's are on Maker too.
Every article card (feed, Maker, Writing) links to the reader at
`/social/a/<naddr>`. The reader shows the newest version the store holds:
an edit is a replacement, and so one more paid write. It reads `title`,
`summary`, `image` (drawn only over `https`, by the visitor's browser, like
any `/social` media) and `published_at` (else `created_at`), and estimates
the reading time at 230 words a minute. The body is Markdown through the
same renderer `/repo` uses: a fixed set of tags, raw HTML shown as text (as
NIP-23 asks), links only to `http(s)`, `mailto`, in-page anchors and
`nostr:` URIs (sent to this app's pages). A Markdown image is a link by its
alt text, not an `<img>`; only the cover is drawn. A `nostr:` mention in the
text is drawn as in a note. A paragraph that is nothing but a mention of a
code snippet (`nostr:nevent1…` of a kind 1337 on this relay) embeds the
snippet's card.

**A series** is a NIP-51 curation set, kind 30004, by the article's author:

```json
{
  "kind": 30004,
  "tags": [
    ["d", "tidepool-devlog"],
    ["title", "tidepool devlog"],
    ["description", "Building an offline tide app in public."],
    ["a", "30023:<author pubkey>:tidepool-why"],
    ["a", "30023:<author pubkey>:tidepool-nsite"],
    ["a", "30023:<author pubkey>:tidepool-cache"]
  ],
  "content": ""
}
```

The reader shows, beside the article, every such set of the author's that
names it: the parts in the set's tag order, "Part N of M", the current part
marked and a part this relay does not hold named by its `d`, unlinked. A set
by someone else is their curation, not the author's series, and is not
shown. A set's encrypted private items cannot be read and do not count.

**Highlights** (kind 9802) whose `a` is the article's address, or whose `e`
is the version being read, fill a rail grouped by passage (white space
collapsed), each counted by distinct people, most first. Up to three passages
that two or more people highlighted are also marked in the text, but only
where the passage is found word for word inside one run of text, outside
code, and without cutting a link or mention in two. A passage that crosses
formatting (half of it in `*emphasis*`), or that a client re-spaced or
trimmed, is in the rail and not marked: finding it would mean matching
across elements, which risks marking the wrong words.

**Reactions** (kind 7, by `a` or `e`) are counted by person per content, as
a post's are; **boosts** (kind 16 with `a`) and **comments** (kind 1111 whose
root `A` is the article, NIP-22) are counted by person and the comments are
listed, the author's and the repo maker's marked.

**A snippet** (kind 1337) has a page at `/social/s/<id>`: its `name`, `l`
language, `extension`, `runtime`, `license` and `dep` tags, its
`description`, and its code as escaped text in a monospace block. Its repo is
its `repo` tag read as a `30617:<pubkey>:<d>` address, an `naddr` or a
`nostr://<npub>/…/<d>` URL (a NIP-05 name there is not resolved: that needs
a fetch), else its `a` tag; a repo on this relay unfurls into its card, and
an `https` URL is a plain link. The code is **not syntax-highlighted**:
`/repo` colours a file with a WebAssembly module, and `/social` pages run no
script; colouring on the server would put syntect in the relay and need a
`style` attribute per run, which the pages' policy refuses.

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
`packages/conformance/suite/harness/social-fixture.ts`; the conformance cases
in `packages/conformance/suite/ui-social.test.ts` seed the same events. Its
media are placeholder images from picsum.photos, which your browser loads.
The seed then loads `suite/harness/devlog-fixture.ts` (a series of articles
with highlights, comments and an embedded snippet), which
`suite/ui-social-devlog.test.ts` seeds too.
