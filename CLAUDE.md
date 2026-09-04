# relay

The TOON Protocol **Nostr relay node**: `@toon-protocol/relay` — a NIP-01
WebSocket read surface, an HTTP `POST /write` surface, and the `startRelay`
launcher/CLI.

Part of the **TOON Protocol** — pay-to-write Nostr over Interledger (ILP),
split into per-team repos. The relay is a plain HTTP/WebSocket app: it speaks
**no** ILP and contains no connector, settlement, or pricing logic. Payment is
enforced upstream by the connector; a request reaching `POST /write` is already
proven paid, so the relay verifies the event signature, stores it, and serves
free reads. It records the payment the connector states on the delivery
(`X-TOON-Payer` / `-Amount` / `-Chain`, connector ADR 0040) without
re-validating it — that statement is the whole trust model.

## Build & test

```
pnpm install
pnpm -r build
pnpm -r test
```

## Deployment

`deploy/` is **the deployment of record** — the live devnet relay box runs it
from this repo (Caddy → connector → relay). One image is published to GHCR on
every green merge to `main`, with a moving `:release` tag the box's Watchtower
follows and an immutable `:sha-*` tag for rollback:

- `ghcr.io/toon-protocol/relay` — the app (`packages/relay/Dockerfile`)

The connector is the **stock** `ghcr.io/toon-protocol/connector` image on an
immutable pin, with `deploy/connector.toml` mounted read-only — the same shape
the store and gas-station bundles use. This repo publishes no connector image.
The pin lives in exactly one place: `deploy/docker-compose.yml`'s `connector`
service `image:`. Because it is immutable, the connector does **not**
auto-deploy: changing it or `connector.toml` needs `git pull && docker compose
up -d` on the box. `deploy/bundle.test.ts` fails the build if a second copy of
the pin appears, if the privacy invariant breaks, or if prices/settlement
drift.

## Cross-repo dependencies

The ILP payment engine is the separate
**[toon-protocol/connector](https://github.com/toon-protocol/connector)** repo
(GHCR image + config reference). **All payment-claim validation lives ONLY in
the connector — never re-implement it here.**

## Shared skills, docs & project context → toon-protocol/toon-meta

Cross-cutting agent skills, docs, and the canonical project context live in
**[toon-protocol/toon-meta](https://github.com/toon-protocol/toon-meta)**:

```
/plugin marketplace add toon-protocol/toon-meta
/plugin install toon-skills@toon-meta
```

Canonical rules/decisions: `toon-meta` → `context/context.md`.

## Publishing

CI publishes via **changesets + `pnpm`** using the org `NPM_TOKEN` secret.
**Never run `npm publish`** (it ships unresolved `workspace:*`).
