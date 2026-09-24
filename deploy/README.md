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
| `bundle.test.ts`     | Fails the build if the privacy invariant, the prices, the settlement deployment, or the pin ever drift.                                                                                |

## Overlays

```bash
# Local, no TLS: Caddy drops out, the reads appear on loopback (the connector
# edge already is there).
docker compose -f docker-compose.yml -f docker-compose.local.yml up -d

# Auto-redeploy: Watchtower follows the relay app's :release tag. Never
# touches Caddy, and has nothing to follow for the pinned connector.
docker compose -f docker-compose.yml -f docker-compose.watchtower.yml up -d
```

## Images

| Image                             | Built by                  | Contents                                                                            |
| --------------------------------- | ------------------------- | ----------------------------------------------------------------------------------- |
| `ghcr.io/toon-protocol/relay`     | `publish-relay-image.yml` | the relay app (`packages/relay`)                                                    |
| `ghcr.io/toon-protocol/connector` | the connector repo        | the stock TOON connector — this repo publishes no connector image and only pins one |

The relay app image publishes `:latest`, a moving `:release` (the Watchtower
target), and an immutable `:sha-<short>` on every green merge to `main`.

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
| `toon-auto-apply.service` / `.timer`               | The systemd pair that runs it every five minutes. Install once — see the root README's "Operate it".                                       |

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

## Secrets

`.env`, `*.key`, `*.secret`, `operator-bearer.token` and `operator-write.keys`
in this directory are gitignored. `connector.toml` is committed and holds
nothing secret: it names key _paths_, and the files themselves are mounted
read-only at runtime.
