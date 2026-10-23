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

## Events and rooms

Launch days, hackathons and the rooms builders talk in. Every NIP here is a
**draft** upstream (NIP-29, -52, -53, -58, -C7, read on 2026-10-23); the
links between them marked *convention* are this guide's, not any NIP's.

| Act | Event | Required tags | Notes |
| --- | --- | --- | --- |
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

A group is made, joined and moderated with NIP-29's kinds 9000–9022 and
exists only on a relay started with `TOON_NIP29_GROUPS=true`. Its metadata
(kinds 39000–39003) is signed by the relay's own key, and `/social`
believes no other. Every message, RSVP, submission and award is one paid
write; reading them is free.

| Path | What it shows | What it reads |
| --- | --- | --- |
| `/social/events` | Calendar events, live now, upcoming and past, with RSVP counts | kinds 31922, 31923; 31925 by `#a` |
| `/social/ev/<npub>/<d>` | One event: when (UTC, with the host's `start_tzid` named), where, hosts, RSVPs, a live banner, its room, badges, and the gallery; once it has ended, submissions whose author holds one of its badges are pinned first | the event; 31925, 30617/1/31990 and 30311 by `#a`; 30009 and 8 by the hosts |
| `/social/ev/<naddr>` | A redirect to the event's page | — |
| `/social/g/<id>` | A room: its chat oldest first (the newest 100), threads, admins, member count, live banner | 39000–39002 by the relay's key; kinds 9, 11 and 30311 by `#h` |

The feed's sidebar shows the next live or upcoming events ("Coming up"), the
header links Events, and a repo page's sidebar links the repo's room.
Buttons that would write (RSVP, open a room) are `nostr:naddr` links a
client opens, and where a chat input would be, the page says it is
read-only.

**What a visitor may read.** A page cannot sign in (NIP-42) for its visitor,
so `/social` reads as a connection that has not authenticated would. A
`private` or `hidden` group shows nothing at all, not even its name; a
`closed` one shows its name and description and none of its messages (this
relay restricts a closed group's reads, which is stricter than NIP-29's
`closed`, a refusal of join requests). The same rule filters every
`/social` page, so a note posted into a private group never reaches the
feed, a repo page or its own `/social/e/<id>`. Both read "This room needs
sign-in in a client". With NIP-29 off, an `h` tag is any tag and there are
no rooms.

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

The script prints the pages to open. A relay started without a connector
accepts `POST /write` directly, which is how the seed writes. The fixture is
`packages/conformance/suite/harness/social-fixture.ts`; the conformance cases
in `packages/conformance/suite/ui-social.test.ts` seed the same events. Its
media are placeholder images from picsum.photos, which your browser loads.

The seed also writes `packages/conformance/suite/harness/social-events-fixture.ts`
(a hackathon that has ended, a live build night, a launch day, a meetup,
their rooms, a private group and tidepool's room), which
`packages/conformance/suite/ui-social-events.test.ts` checks. Its rooms need
`TOON_NIP29_GROUPS=true`, as above; without it the group events are stored
as plain events and `/social/g/…` says the relay has no rooms.
