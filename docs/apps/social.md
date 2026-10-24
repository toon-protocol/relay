# Social over TOON, repos first

A protocol guide for the social side of building in public on Nostr: notes,
launches, stars, follows and talk about repositories. It says **what goes in
the events** a client publishes and how the relay's `/social` pages read them.
It names no client, CLI or library: anything that can sign Nostr events and
pay for writes can follow it.

Sources, in order of authority:

- The NIPs each section names (NIP-01, -02, -10, -18, -22, -23, -24, -25,
  -27, -29, -32, -38, -39, -51, -52, -53, -56, -57, -58, -5A, -68, -71, -75,
  -84, -85, -89, -92, -99, -C0, -C7, -7D), the NIP-82 draft (an open pull
  request), NIP-34 for forks, and TOON's draft NIP-C1
  ([`../nips/nip-c1.md`](../nips/nip-c1.md)). NIP-5A, NIP-89 and NIP-34's
  fork tags were read at nostr-protocol/nips@d47a2ec.
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
| Keep a series | kind 30004 by the articles' author | `d`, `title`, one `a` = `30023:<author>:<d>` per part, in order | NIP-51 curation set; see [Devlogs](#devlogs-series-highlights-and-snippets) |
| Highlight an article | kind 9802 | `a` = the article's `30023:<author>:<d>` (and `p` = author) | NIP-84; content is the passage, word for word |
| Comment on an article | kind 1111 | `A` = the article's address, `K` = `30023`, `P` = author | NIP-22; a kind 7 or 16 on it carries `a` (and `e`) and `k` = `30023` |
| Hold an event | kind 31923 (a time) or 31922 (whole days) | `d`, `title`, `start` (and `end`, `start_tzid`, `D` for 31923) | NIP-52. `p` with a role names people; a role saying `host` or `organizer` makes a host |
| RSVP | kind 31925 | `d`, `a` = the event, `status` `accepted`, `tentative` or `declined` | NIP-52. Counted by person: each key's newest RSVP is its answer |
| Submit to an event | kind 30617, or a ship note (kind 1 with `t` `ship` or `imeta`), or a kind 31990 | `a` = the event | *Convention.* NIP-52's `a` links an event to a calendar, not a project to an event. A maker's ship note that also tags their repo joins the repo's entry |
| Give the event a room | the calendar event itself | `h` = a group id on this relay | NIP-29 lets a group hold any kind; the relay checks the host is a member |
| Define a prize | kind 30009 by a host | `d`, `name`; `a` = the event (or the event `a`-tags the badge) | NIP-58 plus a *convention* tying the badge to the event |
| Award it | kind 8 by the badge's author | `a` = the badge, `p` = each winner | NIP-58. An award by anyone else does not count |
| Stream it | kind 30311 | `a` = the event, or `h` = the room; `status`, `streaming` | NIP-53 plus a *convention* for the link. `live` not updated for an hour reads as ended |
| Chat in a room | kind 9 | `h` = the group id; `q` quotes the message answered | NIP-C7 inside a NIP-29 group |
| Start a room thread | kind 11 | `h`, `title` | NIP-7D inside a NIP-29 group |
| A repo's room | a group whose id is the repo's `d` | — | *Convention.* Its admins (kind 39001) must include an authorized author of the repo (ADR 0002). No NIP has a tag linking a group and a repo, and this relay's group metadata carries only the fields NIP-29 names |

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
site is no demo. A kind 32267's `url` is not a demo either: NIP-82 defines
it as the app's website, so where rule 3 finds one (and no site), the pages
draw it as a **Website** link in the demo's place, labelled the same on the
card, the repo page, a launch and the app's own page.

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
| `/social/r/<npub>/<d>` | The repo as a profile; tabs `/talk`, `/writing`, `/stars`, `/followers`, `/forks`, `/media`, `/snippets`; `?as=<npub>` ranks "Open in…" by that key's follows | `{"#a":[A]}`, `{"kinds":[1111],"#A":[A]}`, kind 7 and 10018 by `#a`, recent 1337s for a `repo` tag, kinds 31989 (`#d` 30617) and 31990 (`#k` 30617) |
| `/social/r/<npub>/<d>/forks` | Forks and remixes as a tree, and repos with the same first commit | up to 2,000 kind 30617s (the `u` value holds a URL, so it is scanned, not indexed) plus `#r` the `euc` commit; the repo's sites' copies by `#A`/`#a`, lookalikes by `#x` |
| `/social/r/<naddr>` | A redirect to the repo's page | — |
| `/social/e/<id>` | One post (hex, `note` or `nevent`) with its replies; a launch when it is one. An article or a snippet redirects to its own page | the event, kind 1 replies by `e`, kind 1111 by `E`, kind 7 reactions by `e` |
| `/social/a/<naddr>` | An article (also `/social/a/<npub>/<d>`): its series, highlights, reactions and comments | the newest 30023 at the address; 30004 by its author with its `a`; 9802 and 7 by `#a` or `#e`; 16 by `#a`; 1111 by `#A` |
| `/social/s/<id>` | A code snippet (hex, `note` or `nevent`) with its repo, reactions and replies | the 1337; kind 7 by `e`; replies as a post's |
| `/social/r/<npub>/<d>/writing` | The repo's Writing tab: every article that tags it, the maker's marked | kind 30023 by `#a` |
| `/social/trending`, `/social/t`, `/social/t/<topic>` | Repos ranked by distinct people, and topics | see [Trending repos and topics](#trending-repos-and-topics) |
| `/social/p/<npub>` | A builder's profile; tabs `/repos`, `/starred`, `/posts` | see [The builder profile](#the-builder-profile) |
| `/social/events` | Calendar events, live now, upcoming and past, with RSVP counts | kinds 31922, 31923; 31925 by `#a` |
| `/social/ev/<npub>/<d>` | One event: when (UTC, with the host's `start_tzid` named), where, hosts, RSVPs, a live banner, its room, badges, and the gallery; once it has ended, submissions whose author holds one of its badges are pinned first | the event; 31925, 30617/1/31990 and 30311 by `#a`; 30009 and 8 by the hosts |
| `/social/ev/<naddr>` | A redirect to the event's page | — |
| `/social/g/<id>` | A room: its chat oldest first (the newest 100), threads, admins, member count, live banner | 39000–39002 by the relay's key; kinds 9, 11 and 30311 by `#h` |
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
people; Stars and Followers list each key once, newest first; Forks draws the
lineage (below); Media lists pictures and videos; Snippets lists code
snippets; Writing lists every article that tags the repo, the maker's marked.
Code goes back to `/repo`,
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
| `/social/t/<topic>` | The repos under one topic, by stars from people other than their makers, each saying whether its makers tagged it or who else labelled it |

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
A label by the repo's own owner or a maintainer is not someone else's claim:
it counts as the makers' own topic, like the `t` tag, and is not listed as a
label.

**Counting is the same on both pages.** A maker never moves their own repo
up: the trending score leaves out the owner and maintainers, and the topic
page orders by stars from people other than them. The star count a row shows
is everyone's, as on a repo card, so a maker's own star is in the number but
not in the order.
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
  post's media) and `bot` (a "bot" chip when it is `true`, the JSON
  boolean only; the same chip follows the key's name in every by-line on
  `/social`: posts, replies, articles). The avatar stays initials, as
  everywhere.
- the NIP-05 name, shown as **unverified**: the relay never fetches the
  `.well-known` document.
- NIP-38 statuses (kind 30315), newest per `d`, `general` first; one that is
  empty or past its `expiration` is not shown. An `a` tag naming a repo
  links it.
- NIP-39 identities from kind 10011 (or the older `i` tags on kind 0), each
  linked to the account and to its proof, marked **unchecked**: a reader
  opens the gist or post to check it. The **Also on** box with its NIP-39
  note opens only when there is such a claim; a kind 0 `website` is shown
  on its own line, as the profile's field, not as a claim.
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

### Devlogs, series, highlights and snippets

NIP-23, NIP-51, NIP-84 and NIP-C0 are all **drafts** upstream; what follows
is how these pages read them today.

**An article** (kind 30023) that `a`-tags a repo is a devlog when its author
is an authorized author of the repo, and a write-up when anyone else wrote
it; both are on the repo's Writing tab, and the maker's are on Maker too.
Every article card (feed, Maker, Writing, a profile's Posts) links to the reader at
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
an `https` URL is a plain link. In a language the shared `ui-highlight`
crate knows (the `l` tag, else the `extension` tag) the code is coloured on
the server as `<span class="hl-…">` runs, styled by the same stylesheet rules
as `/repo`'s blob pages; no `style` attribute and no script, so the pages'
policy is unchanged. Any other language, or code over the size limit, stays
plain escaped text.

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

### Link previews

A `/social` link posted on X, Discord, Slack, Bluesky or anything else that
reads Open Graph unfurls into a card. Every page's `<head>` carries
`og:title`, `og:description` (plain text, cut to 200 characters), `og:type`,
`og:site_name`, `og:url`, `og:image` with its `type`, `width`, `height` and
`alt`, and `twitter:card` (`summary_large_image`), `twitter:title`,
`twitter:description` and `twitter:image`.

The image is a 1200×630 PNG **the relay draws itself**, from its own store,
with no outbound fetch (ADR 0006; ADR 0001 stands). It is text and plain
shapes in the pages' light colours, in the bundled Inter font (SIL Open Font
License 1.1, `crates/ui-preview/fonts/OFL.txt`); it never holds a picture,
so an avatar is initials as on the pages, and a character Inter cannot draw
(CJK, emoji) is left out of the image:

| Image                             | Named by                     | Shows                                                                  |
| --------------------------------- | ---------------------------- | ---------------------------------------------------------------------- |
| `/social/og/r/<npub>/<d>.png`     | the repo page, every tab     | owner, name, description, up to four topics, stars, followers and forks counted as the page counts them |
| `/social/og/p/<npub>.png`         | the profile                  | name, the NIP-05 claim (never called verified), about, repos kept, followers, a `bot` chip for `bot: true` |
| `/social/og/e/<id>.png`           | a ship post                  | "Shipped", the headline, then the repo's name, description and counts |
| `/social/og/a/<npub>/<d>.png`     | an article                   | its title, author and summary                                          |
| `/social/og/site.png`             | every other page             | "TOON Social" and what it is                                           |

A path naming something the relay does not hold is `404`, and so is a post
that is not a ship post (its page names the site card). An image is served
`Content-Type: image/png`, with no authentication and the default policy
(`default-src 'self'`): nothing about a page's policy is loosened for it.

**Cost is bounded.** An image's `ETag` is a hash of what it shows, so it
changes when a count does and a crawler asking again with `If-None-Match`
gets `304`; `Cache-Control: public, max-age=600`. Drawn images are kept in a
cache of at most 512 images and 32 MiB, least recently used out first, and
at most two are drawn at once (on blocking threads, about 20 ms each); a
request past that is `503` with `Retry-After`. Every text is cut before it
is measured, so neither the SVG nor the time to draw it grows with the
events.

**The public origin.** Open Graph wants absolute URLs, so the pages need to
know where they are reached: `TOON_UI_PUBLIC_URL` (an `https` origin), else
the `https` origin of a `wss://` or `https://` `TOON_UI_RELAY_URL`. With
neither, a page still states its title and description but names no image
and no `og:url`, rather than a relative URL a crawler would drop.

## Events and rooms

Launch days, hackathons and the rooms builders talk in. Every NIP here is a
**draft** upstream (NIP-29, -52, -53, -58, -C7, read on 2026-10-23); the
links between them marked *convention* are this guide's, not any NIP's.

The acts are the [Conventions](#conventions) rows from "Hold an event" on;
the pages are in the [paths table](#reading-in-a-browser-the-relays-social-pages).

A group is made, joined and moderated with NIP-29's kinds 9000–9022 and
exists only on a relay started with `TOON_NIP29_GROUPS=true`. Its metadata
(kinds 39000–39003) is signed by the relay's own key, and `/social`
believes no other. Every message, RSVP, submission and award is one paid
write; reading them is free.

The feed's sidebar shows the next live or upcoming events ("Coming up"), the
header links Events, and a repo page's sidebar links the repo's room.
Buttons that would write (RSVP, open a room) are `nostr:naddr` links a
client opens, and where a chat input would be, the page says it is
read-only.

**What a visitor may read.** A page cannot sign in (NIP-42) for its visitor,
so the relay hands every app UI (`/repo` as well as `/social`) the store
as a connection that has not authenticated reads it: the same rules a
session applies, in `crates/relay/src/ui.rs`. A `private` group shows
nothing at all, not even its name: its room is a 404, as a room that does
not exist is, and says a private room needs sign-in in a client. A
`closed` one shows its name and description and none of its messages (this
relay restricts a closed group's reads, which is stricter than NIP-29's
`closed`, a refusal of join requests), and reads "This room needs sign-in
in a client". The same rule filters every page, so a note or issue posted
into a private group never reaches the feed, a repo page on `/social` or
`/repo`, or its own `/social/e/<id>`. Kinds the operator restricts to
signed-in readers (`TOON_AUTH_REQUIRED_KINDS`), and gift wraps when only
their recipients may read them (`TOON_NIP17_RECIPIENT_ONLY`), are left out
the same way. With NIP-29 off, an `h` tag is any tag and there are no
rooms.

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
owner)"**, with each share computed as NIP-57 Appendix G says. A person's
kind 0 with `zap` tags shows the same on their profile, "Tip split (set by
this person)".

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
reports count once. An article page shows the same line for reports naming
the article's address or the version shown, and a profile for reports about
the person: a report whose only target is its `p` tag (one that also names
an `e`, `a` or `x` is about that note, repo or file, not the person). Those
two have no reports list to link to.

With `?as=<npub>`, the feed `/social` (and always the following feed of its
own key) leaves out what that key's NIP-51 mute list (kind 10000) names:
`p` people, `t` hashtags, `word`s and `e` threads, and repos whose owner is
muted or whose topics are muted from "trending today". Only the list's public
tags are read; its encrypted private items stay private to its author's
client. `crates/ui-social/src/mute.rs` is the filter, for any list to reuse.
The same `?as=` applies it elsewhere: `/social/trending` does not count the
acts of muted people and drops repos whose owner or own topic is muted
(the page says how many); `/social/t/<topic>` drops the same repos and the
labels of muted people; a profile's Starred tab drops repos whose owner is
muted, and the profile says when the key's mute list names that person.

## Trying it locally

A dev seed loads a small, realistic community (profiles, two repos and a
Shakespeare app, stars, follows, a ship post with screenshots and a live demo,
an `naddr` mention, a boost, comments, a thread, a highlight, an article, a
picture, a snippet, an issue and a merge) into a relay on this machine:

```bash
cargo build -p relay
TOON_SECRET_KEY=$(printf '1%.0s' {1..64}) TOON_DATA_DIR=/tmp/social-dev \
  TOON_UI_RELAY_URL=ws://127.0.0.1:7100 TOON_UI_NSITE_GATEWAY=https://nsite.lol \
  TOON_NIP29_GROUPS=true target/debug/relay
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
`suite/ui-social-profile.test.ts` and the seed load after it. Forks, remixes,
handlers and an app listing are `suite/harness/lineage-fixture.ts` (its cases
are `suite/ui-social-lineage.test.ts`). Its media are placeholder images from
picsum.photos, which your browser loads. The seed then loads
`suite/harness/devlog-fixture.ts` (a series of articles with highlights,
comments and an embedded snippet), which `suite/ui-social-devlog.test.ts`
seeds too.

The seed also writes `packages/conformance/suite/harness/social-events-fixture.ts`
(a hackathon that has ended, a live build night, a launch day, a meetup,
their rooms, a private group and tidepool's room), which
`packages/conformance/suite/ui-social-events.test.ts` checks. Its rooms need
`TOON_NIP29_GROUPS=true`, as above; without it the group events are stored
as plain events and `/social/g/…` says the relay has no rooms.

The seed also loads `suite/harness/social-apps-fixture.ts`: tidepool as an
app with releases and files, an app set, bounties, a goal, reports and a mute
list. `ui-social-apps.test.ts` seeds the same; open the printed `read as`,
`reports` and `muted feed` links to see the `?as=` lens at work.
