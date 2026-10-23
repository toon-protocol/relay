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
| `/social/e/<id>` | One post (hex, `note` or `nevent`) with its replies; a launch when it is one | the event, kind 1 replies by `e`, kind 1111 by `E`, kind 7 reactions by `e` |
| `/social/apps` | Every app listing here, `?t=<topic>` for one topic, and recent app sets | kind 32267 (NIP-82, **draft**), kind 7 and 30267 by `#a` |
| `/social/app/<npub>/<d>`, `/social/app/<naddr>` | One app: its releases by channel (`?channel=`), files, who vouches, `?as=<npub>` | kinds 32267, 30063 by `#i`, 3063 by id, 7 and 30267 by `#a`, 1984 |
| `/social/apps/sets/<naddr>` | One app set and the apps it lists | kind 30267, then 32267 |
| `/social/bounties` | Bounties by status (`?status=open\|claimed\|resolved`, `?repo=<A>`) and goals | kind 30402 by `#t` `bounty`, 1621 and 1630–1633, 9041 |
| `/social/r/<npub>/<d>/reports`, `/social/app/<npub>/<d>/reports` | The reports on a repo (or an app) and what it ships | kind 1984 by `#a`, `#e`, `#x` |

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
pictures and videos; Snippets lists code snippets. Code goes back to `/repo`,
and `/repo`'s repository header has a Social tab back here.

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

## Apps, bounties, tips and trust

Proposals 15, 16, 18 of the research note, and app listings. Sources: the
open NIP-82 pull request
([nostr-protocol/nips#1336](https://github.com/nostr-protocol/nips/pull/1336),
read at its head `9d959cd`), and NIP-51, -56, -57, -75 and -99 at
nostr-protocol/nips `d47a2ec`.

### Reading as someone: `?as=<npub>`

The pages have no login (ADR 0001), so a page that can be read through
someone's eyes takes `?as=<npub>` in its URL and reads that key's public
lists: its kind 3 follows and its kind 10000 mute list. Anyone can read as
anyone, since those lists are public; nothing is kept.

### Apps (NIP-82, an unmerged draft)

**NIP-82 is not a merged NIP.** It is an open pull request (Zapstore uses
it); its shapes may still change, and the pages say so. A listing is three
kinds, each one paid write:

| Event | Kind | What the pages read |
| --- | --- | --- |
| App | 32267, addressable | `d` (app id, reverse-domain), `name`, `summary`, `icon`, `image`s, `t`s, `url`, `repository`, `a` → `30617:…` (the source repo), `f` platforms, `license`; content is Markdown |
| Release | 30063, addressable | `i` = the app's `d`, `version`, `d` = `<i>@<version>`, `c` channel (`main` when absent), `e` per asset; content is the release notes |
| Asset | 3063 | `i`, `url`, `m` MIME, `x` SHA-256, `size`, `version`, `f`, `variant`, `commit`, `supported_nip` |

- Releases count only when signed by the app's author with `i` equal to its
  `d`. Each channel shows its **newest version** by the draft's Appendix D,
  not its newest event: SemVer precedence, a leading `v` ignored, fallback
  segments on `.`, `-`, `_`, and the draft's own example order
  (`1.0.0-dev < -alpha < -alpha.2 < -beta < -beta.2 < -rc1 < 1.0.0`), which
  ranks `dev < alpha < beta < rc/pre/preview` where plain SemVer would not
  (`crates/ui-social/src/version.rs`).
- An asset row shows platform, MIME type, size, the SHA-256 shortened (full in
  its title) and the commit, linked to the `/repo` commit page when the app
  `a`-tags a repo. It links **Download** only for an `https` `url`;
  otherwise it says to fetch the file by hash from a Blossom server. The relay
  fetches neither the file nor any image: the browser loads `https` icons and
  screenshots under the media policy below.
- **Who vouches** counts distinct people who starred the app (a kind 7 `+`
  with `a` = the app's address; a `k` other than `32267` disqualifies it) or
  put it in a NIP-51 app set (kind 30267). With `?as=`, only the people that
  key follows. The card says whether the signer also keeps the linked repo
  (ADR 0002's authorized authors).
- A repo's page shows **Latest release** when an authorized author's app
  `a`-tags it.

### Bounties and goals

A bounty is a NIP-99 listing tagged as one, naming a repo or an issue:

```json
{
  "kind": 30402,
  "tags": [
    ["d", "southern-stations"],
    ["title", "Southern Hemisphere tide stations"],
    ["t", "bounty"],
    ["price", "50", "USDC"],
    ["e", "<issue id, kind 1621>"],
    ["a", "30617:<owner pubkey>:tidepool"]
  ],
  "content": "What is wanted, in Markdown."
}
```

Its status on `/social/bounties` is **Resolved** when its issue's honoured
status (ADR 0002 rule 3: the owner, a maintainer or the issue's author) is a
kind 1631; else **Claimed** when the listing's NIP-99 `status` is `sold` (or
`claimed`); else **Open**. The price is shown as written; nothing converts or
checks it. A repo's page counts the open bounties naming it.

A goal is a NIP-75 kind 9041 with an `a` tag naming the repo. The page shows
its description and its target (`amount`, in millisats) and **no progress**:
NIP-75 tallies Lightning zap receipts (kind 9735), which TOON does not
produce, and the relay never checks a payment.

### Tips: the gap

There is no tip button, on purpose. The Nostr shape for tipping is NIP-57
zaps, and they are out on TOON: a zap receipt is written by the recipient's
Lightning server, not by the payer, and nothing on TOON carries Lightning. The
relay must not validate a payment either; payment checks live only in the
connector. What would close the gap is a NIP for a **receipt the payer
writes** for an ILP payment to a pubkey, taggable to a repo, which the relay
stores and shows as a claim (or as connector-stated, if the connector states
it) and never verifies. Until then, a repo announcement or app that carries
NIP-57 `zap` split tags shows them as text, **"Tip split (set by the
owner)"**, with each share computed as NIP-57 Appendix G says.

### Reports and mutes

A report is a NIP-56 kind 1984. NIP-56 names the target in an `e`, `p` or `x`
tag with the report type as its third entry; to report a repo or an app, this
guide's convention is the same on an `a` tag:

```json
{ "kind": 1984, "tags": [["a", "30617:<owner pubkey>:tidepool", "malware"], ["p", "<owner pubkey>"]], "content": "Why." }
```

A file is reported by its asset's `e` and its `x` hash. The pages read reports
on a repo, its makers' posts about it, the apps they publish for it, their
releases and files. With `?as=<npub>`, a repo, app or post page shows a banner
when people that key follows reported it (how many, by type) with a link to
the reports list; without it, only a neutral "N reports on this relay" line.
**Nothing is hidden because of a report**: anyone can report anything, and
removal stays the operator's blocklist (`docs/retention.md`). A person's
reports count once.

With `?as=<npub>`, the feed `/social` (and always the following feed of its
own key) leaves out what that key's NIP-51 mute list (kind 10000) names:
`p` people, `t` hashtags, `word`s and `e` threads, and repos whose owner is
muted or whose topics are muted from "trending today". Only the list's public
tags are read; its encrypted private items stay private to its author's
client. `crates/ui-social/src/mute.rs` is the filter, for any list to reuse.

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

The seed then loads `suite/harness/social-apps-fixture.ts`: tidepool as an
app with releases and files, an app set, bounties, a goal, reports and a mute
list. `ui-social-apps.test.ts` seeds the same; open the printed `read as`,
`reports` and `muted feed` links to see the `?as=` lens at work.
