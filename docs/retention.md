# Retention: what the relay stops serving, and why

Three mechanisms decide when an event leaves this relay's read surface. They
are deliberately different in who holds the power.

| Mechanism          | Who decides                 | Reaches                                 |
| ------------------ | --------------------------- | --------------------------------------- |
| NIP-01 replacement | the author's key            | a newer event, same `(pubkey, kind, d)` |
| NIP-40 expiration  | the author, at publish time | events whose stated lifetime has passed |
| NIP-09 deletion    | the author's key            | any of that author's own events         |
| Operator blocklist | this node's operator        | one explicitly named event id           |

The first three are the protocol. The fourth is not, and is scoped as
narrowly as the job allows — see [Unretractable events](#unretractable-events).

## The problem this closed

TOON's discovery used to run on kind:10032 node announces — which node
terminates a destination, its BTP endpoint, its settlement addresses — and
before this work the relay enforced none of the above, so an announce, once
published, was served for as long as the database survived. Live devnet state
at the time:

```
pk=915d2990  g.toon.relay      expired 2026-08-09  still served (186h past its own 10-minute TTL)
pk=31e6a28e  g.drew.relay      expired 2026-08-14  still served
pk=b23599a6  g.toon.swap.sol   no expiration tag   ws://127.0.0.1:3401, key gone
```

The first two said "I am valid for ten minutes" and were handed to clients a
week later. The third is worse: a throwaway proof rig that advertised a
loopback endpoint — an address that resolves to whatever machine _reads_ it —
with no expiry and no surviving key.

**That discovery mechanism is retired**, and this relay no longer publishes an
announce of its own: a node is reached at its URL, and everything about its
paid side is served on the connector's own `GET /ilp` (connector ADR 0046 /
0050). None of that makes this page obsolete, because the relay still _stores
and serves_ whatever its clients write — kind:10032 events among them — and
these are the mechanisms that decide when any of it stops being served. The
announce is simply the example that made the gap visible.

## NIP-40 — expiration

An event carrying `["expiration", "<unix seconds>"]` stops being served at
that timestamp: not from a REQ against history, not from a live fan-out.
A background reaper then deletes it.

| Setting                                 | Default | Meaning                                          |
| --------------------------------------- | ------- | ------------------------------------------------ |
| `TOON_ENFORCE_EXPIRATION`               | `true`  | `false` restores the old serve-forever behaviour |
| `TOON_EXPIRATION_REAP_GRACE_SECONDS`    | `86400` | how long an expired event stays on disk          |
| `TOON_EXPIRATION_REAP_INTERVAL_SECONDS` | `3600`  | sweep interval; `0` disables reaping             |

Parsing **fails open**: an `expiration` tag that is not a plain non-negative
integer is treated as "never expires". A publisher-side typo must not become
an unrecoverable read outage.

### The risk this default carries

Enforcement changes a failure mode. Read this before assuming it is free.

Enforcement turns "a stale event is still served" into "that event is gone
from every read". For most traffic that is exactly what an `expiration` tag
asks for. It matters most for events a **client republishes on a timer** —
anything whose absence means a peer stops being discoverable, rather than
merely stale.

The failure mode to reason about is a publisher that stops publishing: a
crashed refresh loop, a drained payment channel, an operator who forgot the
cron. Before enforcement, its last event stayed readable indefinitely. With
enforcement, it disappears the moment its TTL passes, and any consumer whose
discovery is fail-closed sees the publisher vanish.

That is the intended semantics — something that cannot say it is alive should
not be advertised as alive — but it is a real change in blast radius. Two
things de-risk it:

- **The kill switch is an env var, not a release.** `TOON_ENFORCE_EXPIRATION=false`
  and a restart puts every still-stored event back on the wire.
- **The reap grace makes that reversible.** Serve-time filtering is a
  decision; a `DELETE` is not. The 24h default means an operator who
  discovers enforcement broke something can undo it without having lost the
  data. Shorten it only once you trust the publishers.

Before enabling on a fleet, confirm the publishers you care about are actually
refreshing — compare `created_at` on two reads a few minutes apart:

```bash
websocat wss://relay-ws.devnet.toonprotocol.dev <<< '["REQ","a",{"kinds":[10032]}]'
```

(kind:10032 is a convenient probe because it is the kind most likely to carry
an `expiration` in a devnet corpus, not because this node publishes one.)

## NIP-09 — deletion

A kind:5 event retracts events the **same pubkey** published, named by `e`
(event id) or `a` (`<kind>:<pubkey>:<d-identifier>`) tags. The relay:

- deletes matching events whose `created_at <= ` the request's `created_at`;
- records a tombstone, so a re-publish cannot resurrect the event;
- stores the kind:5 itself, so it can propagate to other relays.

**A deletion request may only ever retract events signed by its own author.**
A kind:5 naming another key's event is not an error and not a partial
success — the named target is simply not deleted. An id tombstone only bites
the pubkey that requested it, so tombstoning an id the relay has never seen
(a legitimate race) cannot be used to pre-block someone else's event.

The `a` path is the one a node wants in practice: it can retract its current
announce without having kept the event id, and immediately publish a fresh
one (the tombstone's watermark is the request's `created_at`, so later events
are unaffected).

## Unretractable events

Neither NIP helps when the author's key is gone. NIP-01 replacement needs the
key. NIP-09 deletion needs the key. If the event also carries no `expiration`
tag — as the `g.toon.swap.sol` announce above does, because the swap node's
boot announce sets no TTL — then **nothing in the protocol will ever remove
it**.

### What an operator can actually do

In order of preference:

1. **Nothing, if the announce is harmless.** A stale announce for a dead node
   costs clients one failed dial. Weigh that against a relay operator taking
   deletion powers.
2. **Fix it at the source.** A publisher that emits an `expiration` tag makes
   its own litter self-clearing. A publisher that passes no TTL is the real
   bug, and fixing it is worth more than any relay-side sweep.
3. **Block the specific event id**, as a last resort.

```bash
TOON_BLOCKED_EVENT_IDS=<64-hex event id>[,<64-hex event id>...]
```

Blocked ids are refused on write, filtered from reads, and swept from the
database at startup.

### The hazard, and how the scope is drawn around it

Anything that lets an operator delete other people's events is a censorship
surface. The blocklist is shaped to be as small a one as possible:

- **Event ids only, never pubkeys.** Blocking a pubkey silences an identity's
  entire past and future output with one line of config. Blocking a 64-hex id
  removes exactly one event the operator had to name explicitly, having
  already seen it. A key that is still alive can simply publish again — so
  this cannot suppress a live participant, only sweep a specific dead
  artifact. A malformed entry is a hard startup error, because the silent
  failure (an operator who believes an event is blocked while the relay keeps
  serving it) is the worse one.
- **Startup configuration, not an API.** There is no admin endpoint and
  nothing network-reachable. Changing the list means restarting with a changed
  deployment — an act that lands in a git history and a deploy log rather than
  in an unlogged HTTP call.
- **Loud.** Every blocked id is printed at boot. A relay withholding events
  should say so on every start.
- **Local.** The block lives in one relay's configuration. Other relays
  serving the same event are unaffected, which is the correct outcome: this is
  an operator declining to carry an artifact, not a protocol-level retraction.

If you find yourself wanting to block by pubkey, or wanting a runtime API to
do it, that is the signal to go back to option 2.
