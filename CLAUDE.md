# relay

The TOON Protocol **Nostr relay node**: `@toon-protocol/relay` (NIP-01 WebSocket reads + an HTTP `POST /write` surface + the `startRelay` launcher/CLI) and `@toon-protocol/bls` (standalone BLS).

Part of the **TOON Protocol** — pay-to-write Nostr over Interledger (ILP), split into per-team repos. The relay is a plain HTTP/WebSocket app: it speaks **no** ILP and contains no connector/settlement/pricing logic. Payment is enforced upstream by an external terminator (the connector); a write reaching `POST /write` is already proven paid, so the relay just verifies the event signature, stores it, and serves free Nostr WS reads.

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
- `relay` and `bls` are co-located workspace packages. `relay` no longer depends on `@toon-protocol/{core,sdk,connector}`; `bls` still consumes them from **npm** (pinned semver).
- The ILP payment engine is the separate **[toon-protocol/connector](https://github.com/toon-protocol/connector)** repo (npm `@toon-protocol/connector` + Docker image). **All payment-claim validation lives ONLY in the connector — never re-implement it here.** The relay trusts that any request reaching `POST /write` was already proven paid.
- The `relay` Docker image is built + pushed to GHCR by `.github/workflows/publish-relay-image.yml` (`ghcr.io/toon-protocol/relay:latest`).

## Publishing
CI publishes via **changesets + `pnpm`** using the org `NPM_TOKEN` secret. **Never run `npm publish`** (it ships unresolved `workspace:*`).
