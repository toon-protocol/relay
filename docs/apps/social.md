# Social over TOON, repos first

A protocol guide for the social side of building in public on Nostr: notes,
launches, stars, follows and talk about repositories. It says **what goes in
the events** a client publishes and how the relay's `/social` pages read them.
It names no client, CLI or library: anything that can sign Nostr events and
pay for writes can follow it.

Sources, in order of authority:

- The NIPs each section names (NIP-01, -02, -10, -18, -22, -23, -24, -25,
  -27, -32, -38, -39, -51, -56, -58, -5A, -68, -71, -84, -85, -89, -92, -C0,
  -7D), and TOON's draft NIP-C1 ([`../nips/nip-c1.md`](../nips/nip-c1.md)).
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
| Pin your repos | kind 10617 | one `a` per `A` you own or maintain, in order | GitWorkshop's list, **not in the NIPs**; a profile shows the first six |
| Say what you are shipping | kind 30315, `d` = `general` | `a` = `A`; optionally `r`, `expiration` | NIP-38 status (draft) |
| Claim an account | kind 10011 | `i` = `<platform>:<identity>`, proof | NIP-39 (draft): `github`, `twitter`, `mastodon`, `telegram` |
| Give a badge | kind 30009 by the issuer, then kind 8 | award: `a` = `30009:<issuer>:<d>`, `p` = holder | NIP-58 (draft); the holder lists `a` and `e` pairs in kind 10008 to show it |
| Mark a bot | kind 0 `"bot": true` | — | NIP-24 |

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
| `/social/p/<npub>` | A builder's profile; tabs `/repos`, `/starred`, `/posts` | see [The builder profile](#the-builder-profile) |

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

**Trending today.** The feed sidebar's top five of `/social/trending`'s
ranking over the last 24 hours (below); on a following feed it is weighted by
that key. Under it, a Topics box links the topics with the most repos.

### Trending repos and topics

| Path | What it shows |
| --- | --- |
| `/social/trending` | Repos ranked by distinct people in a window: `?window=day`, `week` (the default) or `month` (the last 24 hours, 7 or 30 days) |
| `/social/trending?as=<npub>` | The same, weighted by that key's kind 3 follows and hiding what they reported |
| `/social/trending?topic=<t>` | Only repos under one topic |
| `/social/trending?by=<npub>` | One NIP-85 provider's signed ranking, when the store holds one |
| `/social/t` | Every topic, with the number of distinct repos under it |
| `/social/t/<topic>` | The repos under one topic, by stars, each saying whether its owner tagged it or who labelled it |

**The score**, printed on the page under "How this is ranked": the number of
distinct pubkeys that, inside the window, did any of these to the repo `A`.

| Act | Event |
| --- | --- |
| starred | kind 7, content `+`, `a` = `A`, `k` = `30617` (NIP-25) |
| boosted | kind 16, `a` = `A`, `k` = `30617` (NIP-18) |
| commented | kind 1111 with `A` = `A` (NIP-22) |
| followed | kind 10018 whose `a` tags include `A`, written in the window (NIP-51) |
| forked | kind 30617 whose `u` tag is `A` (NIP-34) |
| posted | kind 1 with `a` = `A` |

- A key counts once per repo, however many acts it wrote; NIP-09 deletions
  have already removed their events from the store.
- The repo's owner and maintainers (ADR 0002) never count for their own repo.
- With `?as=<npub>`, each person that key's newest kind 3 lists counts **3×**
  (score = people + 2 × followed people), and a repo that the key or anyone
  it follows reported (kind 1984 with `a` = `A`, NIP-56) is hidden; the page
  says how many were.
- **Follows and forks are replaceable.** The store keeps only each key's
  newest kind 10018 list and newest announcement per `d`, so the relay knows
  who follows a repo *now* and when that list was last saved, not when the
  repo was added. A follow counts when the current list names the repo and was
  written in the window: a key that followed long ago and saves its list today
  for another reason counts again, and an unfollow leaves no trace. A fork
  counts the same way, by its announcement's `created_at`.
- Each row draws seven bars, one per seventh of the window (about 3½ hours,
  one day, or about 4 days), each the distinct people in it, scaled to the
  row's busiest bar. The bars are height classes in the stylesheet, not inline
  styles.
- Only this relay's events are read (ADR 0001), so this is "trending here".
  Every write is paid, which puts a floor under spam; the amount is not stored
  with the event, so money does not change the rank.

**Topics.** A repo is under topic `<t>` (compared lower-cased, a leading `#`
dropped) when its owner's announcement has `["t", "<t>"]`, or when anyone
publishes a NIP-32 label in the `#t` namespace naming it:

```json
{
  "kind": 1985,
  "tags": [
    ["L", "#t"],
    ["l", "built-with:claude", "#t"],
    ["a", "30617:<owner pubkey>:surfcast"]
  ],
  "content": "Shakespeare builds on Claude."
}
```

`#t` is NIP-32's own convention for "associate the target with this standard
tag" (an `L` starting with `#`), so a label in it means exactly what the
owner's `t` tag means and needs no new namespace. Labels in any other
namespace, and `l` tags without the `#t` mark, are not topics. A label is the
labeller's claim, paid for with one write: the topic page names each labeller.
Topic chips on repo cards and repo pages link to `/social/t/<topic>`. NIP-32
is a draft NIP.

**Other rankings (NIP-85).** When the store holds kind 30384 trusted
assertions whose `d` is a repo address and whose `rank` is a whole number, the
trending page lists each provider (the signing key) with how many repos it
ranks, and `?by=<npub>` shows its ranking, highest `rank` first. With `?as=`,
providers that key names for `30384:rank` in its kind 10040 are marked as
trusted and listed first. When there are no such assertions the box is not
drawn. The relay neither computes nor checks these ranks. NIP-85 is a draft
NIP.

### The builder profile

`/social/p/<npub>` (an `nprofile` or 64 hex works too) is what a key says
about itself and what it built. Every author name and avatar on `/social`
links here, and `/repo` links a repo's owner and maintainers here when
`/social` is on.

**The column on the left** reads the key's own events:

- kind 0: name (`display_name`, else `name`), `about`, `website`, and the
  NIP-24 `banner` (an `https` image the visitor's browser loads, like a
  post's media) and `bot` (a "bot" chip when it is `true`). The avatar stays
  initials, as everywhere.
- the NIP-05 name, shown as **unverified**: the relay never fetches the
  `.well-known` document.
- NIP-38 statuses (kind 30315), newest per `d`, `general` first; one that is
  empty or past its `expiration` is not shown. An `a` tag naming a repo
  links it.
- NIP-39 identities from kind 10011 (or the older `i` tags on kind 0), each
  linked to the account and to its proof, marked **unchecked**: a reader
  opens the gist or post to check it.
- followers: distinct keys whose kind 3 names this key; following: the `p`
  tags of its own kind 3, linking to its following feed; repos followed: the
  `a` tags of its kind 10018.
- NIP-58 badges: the pairs in its kind 10008 (or the older kind 30008
  `profile_badges`), each shown only when the `e` award is a kind 8 signed
  by the definition's issuer that names this key and that definition.
- A "Follow in your client" link, `nostr:<npub>`. The page writes nothing.

There is **no "agents this person runs" section**: no NIP lets a key say
which other keys it operates in a way a reader can check, so the page does
not guess. A key that runs as an agent says so itself with `bot`.

**The tabs**:

| Tab | Path | What it lists |
| --- | --- | --- |
| Pinned | `/social/p/<npub>` | the key's kind 10617 `a` tags in order, only repos it owns or maintains, at most six; without a list, its six most-starred owned repos, and the page says so. Then the activity graph and recent activity |
| Repositories | `/repos` | 30617s it announced, then repos whose owner's newest announcement names it in `maintainers` (ADR 0002), each marked owner or maintainer |
| Starred | `/starred` | repos it starred (kind 7 `+` with `a` and `k` = `30617`), newest first |
| Posts | `/posts` | its notes that start a conversation (kind 1) and articles (kind 30023), the newest 40 |

**The activity graph** is 53 weeks of days, a week a column, Sunday at the
top, ending today. A square counts the **regular** events the key signed
that day of these kinds: patches (1617), pull requests (1618) and their
updates (1619), issues (1621), status changes (1630–1633), comments
(1111), notes (1) and NIP-C1 CI results (9841, 9842; a TOON draft). Repo
state (30618) is not counted: it is addressable, so the store keeps only
the newest per repo and pushes cannot be rebuilt from it. Shades are
quarters of the key's busiest day. The heading reads "N contributions
across M repos", M being the distinct repos those events name in an `a`
or `A` tag. The page prints this rule under the graph. Recent activity
lists the ten newest of the same events, each linking to its issue or pull
request on `/repo` (or its post on `/social`).

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
A profile page loads one image the person chose: their `banner`.
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

The script prints the pages to open, Maya's profile and a bot's among
them. A relay started without a connector
accepts `POST /write` directly, which is how the seed writes. The fixture is
`packages/conformance/suite/harness/social-fixture.ts`; the conformance cases
in `packages/conformance/suite/ui-social.test.ts` seed the same events. The
seed then adds `suite/harness/trending-fixture.ts`, a month of stars, forks,
topic labels, a report and NIP-85 ranks for `/social/trending` and `/social/t`
(its cases are `suite/ui-social-trending.test.ts`). The profile's extra
events (a banner, a status, identities, pins, a badge, a year of activity, a
bot) are `suite/harness/profile-fixture.ts`, which
`suite/ui-social-profile.test.ts` and the seed load after it. Its media are placeholder images from picsum.photos, which your browser loads.
