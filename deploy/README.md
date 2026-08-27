# deploy

The files that run a TOON relay node. Start with the
[root README](../README.md) — it walks through DNS, keys, `.env`, and the
first `docker compose up`. This page is the reference for what each file is.

| File                 | What it is                                                                                                                                         |
| -------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| `docker-compose.yml` | The node: Caddy (TLS) → connector (payments) → relay (Nostr). Caddy is the only service that publishes a port.                                     |
| `connector.toml`     | The connector's whole configuration — routes, prices, settlement, and what this node says about itself. Baked into the image.                      |
| `Dockerfile`         | The `relay-connector` image: the published connector plus `connector.toml`. Its `ARG CONNECTOR_TAG` is the only place a connector build is pinned. |
| `Caddyfile`          | TLS for the two public hostnames. Two lines of actual routing.                                                                                     |
| `.env.example`       | Copy to `.env`. Four required values; the rest have defaults.                                                                                      |
| `bundle.test.ts`     | Fails the build if the privacy invariant, the prices, the settlement deployment, or the pin ever drift.                                            |

## Overlays

```bash
# Local, no TLS: Caddy drops out, the edge and reads appear on loopback.
docker compose -f docker-compose.yml -f docker-compose.local.yml up -d

# Auto-redeploy: Watchtower follows the two :release tags. Never touches Caddy.
docker compose -f docker-compose.yml -f docker-compose.watchtower.yml up -d
```

## Images

| Image                                   | Built by                            | Contents                                                 |
| --------------------------------------- | ----------------------------------- | -------------------------------------------------------- |
| `ghcr.io/toon-protocol/relay`           | `publish-relay-image.yml`           | the relay app (`packages/relay`)                         |
| `ghcr.io/toon-protocol/relay-connector` | `publish-relay-connector-image.yml` | the pinned connector + this directory's `connector.toml` |

Both publish `:latest`, a moving `:release` (the Watchtower target), and an
immutable `:sha-<short>` on every green merge to `main`.

The connector pin lives in exactly one place: `Dockerfile`'s
`ARG CONNECTOR_TAG`. The publish workflow passes no build-arg override, so
whatever ships to GHCR is what that file names. Bumping it is a reviewed
commit that carries the config change alongside it — which is the point of
baking the config rather than mounting it.

## Secrets

`.env`, `*.key` and `*.secret` in this directory are gitignored. Nothing
secret is ever baked into the published image: `connector.toml` names key
_paths_, and the files themselves are mounted read-only at runtime.
