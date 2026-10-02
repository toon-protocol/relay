# deploy

The files that run a TOON relay node. Start with the
[root README](../README.md) — it walks through DNS, keys, `.env`, and the
first `docker compose up`. This page is the reference for what each file is.

| File                 | What it is                                                                                                                                                                             |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `docker-compose.yml` | The node: Caddy (TLS) → connector (payments) → relay (Nostr). Caddy is the only service reachable off-box, and the connector's `image:` is the only place a connector build is pinned. |
| `connector.toml`     | The connector's whole configuration — routes, prices, settlement, and what this node says about itself. Mounted read-only into the stock connector image.                              |
| `Caddyfile`          | TLS for the two public hostnames. Two lines of actual routing.                                                                                                                         |
| `.env.example`       | Copy to `.env`. Four required values; the rest have defaults.                                                                                                                          |
| `docker-compose.shared-edge.yml` | Overlay: this node behind the devnet host's shared edge instead of its own Caddy. See "The shared-edge overlay" below.                                                     |
| `bundle.test.ts`     | Fails the build if the privacy invariant, the prices, the settlement deployment, the pin, or the shared-edge overlay's own properties ever drift.                                      |

## Overlays

```bash
# Local, no TLS: Caddy drops out, the reads appear on loopback (the connector
# edge already is there).
docker compose -f docker-compose.yml -f docker-compose.local.yml up -d

# Auto-redeploy: Watchtower follows the relay app's :release tag. Never
# touches Caddy, and has nothing to follow for the pinned connector.
docker compose -f docker-compose.yml -f docker-compose.watchtower.yml up -d
```

The two above are run by naming both files on the command line. The
shared-edge overlay below is turned on the other way — named in `.env`,
because `auto-apply.sh` needs to see it too (see the next section).

### The shared-edge overlay (toon-protocol/relay#166, infra ADR 0001)

The devnet is moving onto one host behind one shared Caddy edge
(`toon-protocol/infra#24`): the relay, store, gas station, workload gateway
and faucet nodes all share it. **Every node keeps its own connector, keys and
hostnames — only its own TLS front goes away.** This node's own `caddy` is the
one thing this overlay removes.

Turn it on in `.env`, not on the command line — `auto-apply.sh` (and an
operator's own `docker compose ps`) need to see it too:

```bash
# .env
COMPOSE_FILE=docker-compose.yml:docker-compose.shared-edge.yml
```

A `.env` with no `COMPOSE_FILE` line runs `docker-compose.yml` alone, exactly
as it does today — this overlay changes nothing until an operator opts in, so
it can merge and ride the box's auto-apply timer while the box is still on its
own Linode, before the edge or the `edge-relay` network exist anywhere
(`bundle.test.ts` holds this: the base file carries no `mem_limit` of its own,
and every property below is asserted against the overlay file, never the
base). `EDGE_HOST`, `READ_HOST` and `ACME_EMAIL` stay required in `.env` even
with the overlay on — Compose still interpolates a disabled service's
`environment:` before it filters the service out — but a disabled Caddy never
reads them again.

What the overlay does:

- **Disables `caddy`** (`profiles: ['never']` — the same never-activated
  sentinel `docker-compose.local.yml` already uses for this service), and
  `docker-compose.watchtower.yml`'s
  `watchtower` too, the same way, if that overlay is also named in
  `COMPOSE_FILE`. Nothing about auto-redeploy changes; this only stops the
  edge overlay's own presence from being read as "and now also run
  Watchtower". List `docker-compose.watchtower.yml` after this file in
  `COMPOSE_FILE` if a box wants both.
- **Joins `connector` and `relay` to the external `edge-relay` network** —
  ONE network per node, not a flat network every node on the host shares
  (infra#24 creates one such network per node: `edge-relay`, `edge-store`,
  `edge-gas`, `edge-gateway`, `edge-faucet`; only Caddy itself joins all
  five). This limits reachability to the edge alone — on a flat network any
  node's containers could otherwise reach any other node's connector
  operator surface. `connector` and `relay` join under the stable aliases
  infra#24's edge config expects — see the table below — and both keep the
  bundle's own default network too (`networks: {default: {}, edge-relay:
  {...}}`), so they still reach each other exactly as they do today.
- **Adds a `mem_limit` to every service**, including the now-disabled `caddy`.
  The `connector` (`64m`) and `relay` (`192m`) numbers are **measured**, not
  guessed: the devnet host turned out to be a 1 GB Linode nanode, not the 2 GB
  host earlier guesses assumed, and idle `docker stats` taken against it on
  2026-09-25 (`toon-protocol/infra#25` step 2) showed the connector at 2 MB and
  the relay at 46 MB — each limit is set to roughly 4x its measured idle
  footprint. `caddy`'s `64m` stays as it was: it isn't running yet to measure,
  and it stays disabled here regardless.

#### The alias:port table — what infra#24's edge config is written from

Worked out from `deploy/Caddyfile`, which is this node's own TLS front today
and still routes both hostnames identically once the shared edge replaces it:

| Hostname | `edge-relay` alias | Container | Port | What's there |
| --- | --- | --- | --- | --- |
| `proxy.relay.devnet` (`EDGE_HOST`) | `relay-proxy` | `connector` | `3000` | The connector's client edge — paid writes, the x402 greeting, and the free `GET /ilp` self-description. |
| `relay-ws.devnet` (`READ_HOST`, WebSocket) | `relay-ws` | `relay` | `7100` | The relay's free NIP-01 WebSocket reads, and on the same port its NIP-11 information document (`GET` with `Accept: application/nostr+json`). |

**What Caddy does beyond plain reverse proxying, that the edge must replicate:**

- **WebSocket upgrade passthrough on `relay-ws`**, with no extra config —
  Caddy's `reverse_proxy` recognizes a `Connection: Upgrade` request and
  proxies the upgraded connection transparently. The edge's proxy for this
  route must do the same, or every read breaks.
- **The standard `reverse_proxy` forwarded headers** on both routes:
  `X-Forwarded-For`, `X-Forwarded-Proto` and `X-Forwarded-Host`. Nothing here
  reads them today, but nothing downstream should ever be handed a scheme or
  origin it did not receive.
- **Nothing else.** Both routes in `Caddyfile` are one `reverse_proxy` line
  each — no path rewriting, no header stripping, no auth, no rate limiting.
  Payment gating lives in the connector; ephemeral-write rate limiting lives
  in the relay. TLS termination and certificate renewal are Caddy's job
  today and infra#24's edge's job once this node sits behind it — neither is
  something the edge "replicates" from this file, since this file no longer
  does it once the overlay is on.

`docker-compose.yml`'s write port (`3100`) is deliberately absent from this
table, from `Caddyfile`, and from `edge-relay`: it is the payment-oblivious
surface, and it must never be reachable except from the connector beside it.

## Images

| Image                             | Built by                  | Contents                                                                            |
| --------------------------------- | ------------------------- | ----------------------------------------------------------------------------------- |
| `ghcr.io/toon-protocol/relay`     | `publish-relay-image.yml` | the relay app (`crates/relay`, the Rust relay)                                      |
| `ghcr.io/toon-protocol/connector` | the connector repo        | the stock TOON connector — this repo publishes no connector image and only pins one |

On every green merge to `main`, once the conformance suite has passed against
the build, the relay app image publishes `:latest`, a moving `:release` (what a
box follows), and two immutable tags: `:rust-<handle>`, a date and that day's
ordinal, which is also the version the relay reports on `/health` and in its
information document, and `:rust-sha-<short>`.

A box takes a moved `:release` when something pulls it. With the Watchtower
overlay that is within about a minute. Without it, `auto-apply.sh` pulls only
when it has a merge to apply, and it usually applies a merge before that
merge's image has been built, tested and pushed, so such a box runs each build
from the next merge on. `docker compose pull relay && docker compose up -d
relay` takes the current one by hand.

### Rolling back to the TypeScript relay

`:release` has been the Rust relay since #205. It replaced the TypeScript relay
as an image swap: same ports, same `/data` volume, same environment, same user
id, and a database either relay opens as the other left it. So the rollback is
an image swap back. The last TypeScript build is an immutable tag that nothing
publishes over:

```bash
# deploy/.env
RELAY_IMAGE=ghcr.io/toon-protocol/relay:sha-7b6bab5
```

```bash
docker compose up -d relay
docker compose exec relay wget -q -O- http://127.0.0.1:3100/health   # "version":"2.3.1"
```

Only the relay container is recreated; the connector and Caddy are not
touched. On the devnet box this took about six seconds to healthy in each
direction (#204). `.env` is not in the repository, so `auto-apply.sh` keeps the
pin: it still pulls and runs `up -d` on every merge, which changes nothing for
a tag that never moves. The box stays on that build until the line is changed.

To return to the Rust relay, set the line back to
`ghcr.io/toon-protocol/relay:release` (or delete it; that is the default) and
pull, since `up -d` alone does not fetch a moved tag:

```bash
docker compose pull relay && docker compose up -d relay
```

`packages/conformance/soak/box.mjs` checks a node from outside before and
after either swap; see [the soak README](../packages/conformance/soak/README.md).

A rollback also puts back what the Rust relay changed on purpose, listed in
#185's compatibility contract. Two an operator can meet: the TypeScript relay
answers a filter with every stored match rather than at most 500, and it
accepts `RELAY_DEV_MODE=true`, which skips signature verification, where the
Rust relay refuses to start.

The TypeScript image is no longer published, so `sha-7b6bab5` is the newest it
will ever be. Putting TypeScript back under `:release` for every stack, rather
than one box, is reverting the commit that flipped it, which is possible only
until the TypeScript source is removed (#206).

The connector pin lives in exactly one place: `docker-compose.yml`'s
`connector.image`, an immutable `rust-sha-` tag. Bumping it is a reviewed
commit that carries any `connector.toml` change alongside it, and the box
takes both with one `git pull` — so a connector can never reach a box ahead of
the config it needs.

This bundle used to publish a derived `ghcr.io/toon-protocol/relay-connector`
image — the stock connector with `connector.toml` COPYed in — and follow its
moving `:release` tag. That is gone (owner decision, 2026-08-28): the immutable
pin plus a mounted config gives the same "build and config move together"
property, and it makes the relay's connector service byte-identical in shape to
the store and gas-station bundles. **A consequence worth stating: the connector
no longer auto-deploys.** Watchtower cannot move an immutable tag, so a
connector or config change is `git pull && docker compose up -d` on the box.
The relay app still auto-deploys.

## Following connector releases

| File                                               | What it is                                                                                                                                 |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `../.github/workflows/adopt-connector-release.yml` | Watches the connector repo for a cut release, boots it against this repo's own `connector.toml`, and opens (and auto-merges) the pin bump. |
| `auto-apply.sh`                                    | On the box: fast-forwards `main`, re-renders if applicable, `compose up -d`, requires the connector to come back healthy, and retries a failed render or apply on every run until it is fixed. |
| `toon-auto-apply-relay.service` / `.timer`         | The systemd pair that runs it every five minutes, named per-node (shared contract v2) so several nodes can share one host. Install once — see the root README's "Operate it", and "Migrating an existing box to the per-node unit names" below for a box that already runs the old names. |

`auto-apply.sh` lives in the repository it applies, which has one consequence
worth knowing: a box sitting on a commit from **before** the script existed
cannot pull itself forward — `systemd` would be pointing `ExecStart` at a file
that is not there yet. Fast-forward that box by hand once and it takes over
from there. The same applies if a future commit ever moves or renames the
script: the box it is running on needs one manual `git pull` to pick up its own
replacement. Neither is reachable by going forwards, only by rewinding a box
behind the change.

The split is deliberate: the workflow decides **what** to run and proves it
accepts this node's committed config first (connector ADR 0041 Decision 1);
the box decides **when** to apply, by pulling. Nothing outside the box can
make the box deploy, which is the posture connector ADR 0068 settled.

### Migrating an existing box to the per-node unit names

Shared contract v2 renames the systemd units from `toon-auto-apply.*` to
`toon-auto-apply-relay.*` (lock `/var/lock/toon-auto-apply-relay.lock`), so
several nodes can share one host once each has its own timer and its own
lock, instead of one name and one lock every node on a box would collide on.
**An existing single-node Linode does not need to do anything right away:**
its already-installed `toon-auto-apply.service`/`.timer` still point at the
same `deploy/auto-apply.sh` path, unaffected by this repo renaming the unit
FILES it ships, so that box keeps applying exactly as it does today.

Do the one-time swap at cutover instead (`toon-protocol/infra#25`), once this
box is about to share the host behind the shared edge:

```bash
sudo systemctl disable --now toon-auto-apply.timer
sudo rm -f /etc/systemd/system/toon-auto-apply.{service,timer}
sudo cp /root/relay/deploy/toon-auto-apply-relay.{service,timer} /etc/systemd/system/
sudo systemctl daemon-reload && sudo systemctl enable --now toon-auto-apply-relay.timer
systemctl list-timers toon-auto-apply-relay.timer     # when it next fires
```

### How updates arrive: a failed render or apply is retried, never sat on

**A render or apply failure is retried, and reported, forever — never
silently sat on (TOON_Network#164, porting TOON_Network#160).** Before this,
`auto-apply.sh` fast-forwarded the checkout and only then pulled and applied
— so a `docker compose pull`/`up -d` failure after a good fast-forward (a
newly-required `.env` variable, since this bundle has no `render.sh` and
interpolates `.env` straight into `docker-compose.yml`, or a pull that fails)
left the box on the new commit with the OLD containers still running, and the
NEXT run's `git fetch` brought back nothing new, so `LOCAL = REMOTE` alone
read as "nothing to do" and it exited 0 silently: one red apply, then green
forever on an unverified box.

The fix is `deploy/.applied` (gitignored). Once the pull, `up -d` and the
health wait have all succeeded, `auto-apply.sh` records the commit it just
applied there. The *next* run compares `HEAD` to `.applied`, not to whatever
`git fetch` just brought back — so a failure anywhere in that chain leaves
`.applied` naming the OLD commit, and the very next timer tick treats that as
work to do even though the fetch brings back nothing new. It fails the same
way, by the same name, on every run — `systemctl status` and the journal keep
showing it — until whatever failed (most often a newly-required `.env`
variable; `.env.example` lists every one) is fixed and a run finally succeeds
and rewrites `.applied`.

On a box with no `deploy/.applied` yet — an existing box's first run under
this check, or one where the file was lost — that absence is read as
*needing* an apply, not as "must already be applied": the run re-applies,
re-verifies and writes `.applied` once everything reports healthy. That run
is a harmless no-op if the box was already caught up, which is why treating a
missing file this way is the safer of the two: the box's first-ever apply IS
this script's first run, and it should prove itself exactly like every later
one does.

### A merged config change restarts the connector

**A merge that changes only `connector.toml` restarts the connector (#173).**
`connector.toml`, the keys and the operator files are bind-mounted, and
`up -d` recreates a container only for a changed image or service definition,
never for changed bytes behind a bind mount. So a config-only merge used to be
"applied; connector healthy" while the connector went on serving the config it
had started with — #171 and #172 both sat unloaded on the devnet until a hand
restart.

`deploy/.connector-inputs` (gitignored) holds a fingerprint of every file the
connector mounts, as they were when it last started and came back healthy.
After `up -d`, `auto-apply.sh` compares the files on disk to that record and,
when they differ, restarts the connector — and only the connector — and waits
for it to be healthy again. It restarts even when `up -d` has just recreated
the container: a second start in a merge that bumps the image and changes the
config is cheaper than trusting a container id read before `up -d`, where one
misread would record the new config as loaded when it never was. A missing
record counts as a change, so a box's first run under this check restarts a
running connector once. Healthy is proof the new file is live: the connector
refuses to start on a config it cannot load.

The same comparison runs on every timer tick, not just when there is a commit
to apply, so a key or operator file changed by hand on the box (gitignored,
so it never arrives in a merge) is loaded on the next run too.

## Secrets

`.env`, `*.key`, `*.secret`, `operator-bearer.token` and `operator-write.keys`
in this directory are gitignored. `connector.toml` is committed and holds
nothing secret: it names key _paths_, and the files themselves are mounted
read-only at runtime.
