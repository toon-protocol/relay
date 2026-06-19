# relay

The TOON Protocol **Nostr relay node**: `@toon-protocol/relay` (NIP-01 WebSocket relay + the Business Logic Server that gates paid writes + the `startRelay` launcher/CLI) and `@toon-protocol/bls` (standalone BLS). The `town` package is still present here pending the `town → relay` merge (`startTown` → `startRelay`).

Part of the **TOON Protocol** — pay-to-write Nostr over Interledger (ILP), split into per-team repos. A paid write arrives from the connector via localDelivery (`POST /handle-packet`) already validated; the relay just runs business logic (store the event) and accepts/rejects. Reads are free Nostr WS.

## Build & test
```
pnpm install
pnpm -r build
pnpm -r test
```

## Shared skills, docs & project context → toon-protocol/toon-meta
Cross-cutting agent skills, docs, and the canonical project context live in **[toon-protocol/toon-meta](https://github.com/toon-protocol/toon-meta)**. Load the shared skills:
```
/plugin marketplace add toon-protocol/toon-meta
/plugin install toon-skills@toon-meta
```
Canonical rules/decisions: `toon-meta` → `_bmad-output/project-context.md`.

## Cross-repo dependencies
- Consumes `@toon-protocol/{core,sdk}` from **npm** (pinned semver); `relay`/`bls`/`town` are co-located workspace packages.
- The ILP payment engine is the separate **[toon-protocol/connector](https://github.com/toon-protocol/connector)** repo (npm `@toon-protocol/connector` + Docker image). **All payment-claim validation lives ONLY in the connector — never re-implement it here.** The relay trusts that a forwarded packet was already proven paid.
- Image-publish workflow (the `relay` Docker image) is a follow-up carved from the monorepo `publish-townhouse-images.yml`.

## Publishing
CI publishes via **changesets + `pnpm`** using the org `NPM_TOKEN` secret. **Never run `npm publish`** (it ships unresolved `workspace:*`).
