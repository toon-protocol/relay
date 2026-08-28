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
| `auto-apply.sh`                                    | On the box: fast-forwards `main`, re-renders if applicable, `compose up -d`, and requires the connector to come back healthy.              |
| `toon-auto-apply.service` / `.timer`               | The systemd pair that runs it every five minutes. Install once — see the root README's "Operate it".                                       |

The split is deliberate: the workflow decides **what** to run and proves it
accepts this node's committed config first (connector ADR 0041 Decision 1);
the box decides **when** to apply, by pulling. Nothing outside the box can
make the box deploy, which is the posture connector ADR 0068 settled.

## Secrets

`.env`, `*.key`, `*.secret`, `operator-bearer.token` and `operator-write.keys`
in this directory are gitignored. `connector.toml` is committed and holds
nothing secret: it names key _paths_, and the files themselves are mounted
read-only at runtime.
