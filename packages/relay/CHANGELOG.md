# @toon-protocol/relay

## 2.0.0

### Major Changes

- b8ec120: refactor(relay)!: make the relay a pure HTTP/WebSocket app — remove all ILP/connector logic and dependencies

  The relay no longer speaks ILP or embeds a connector. Payment is enforced
  entirely upstream by an external terminator, so the relay is now just:
  - free NIP-01 WebSocket reads (`TOON_RELAY_PORT`, default 7100), and
  - an HTTP `POST /write` surface plus `GET /health` (`TOON_BLS_PORT`, default 3100) that trusts injected `X-TOON-*` headers without re-validating payment.

  **Removed** (BREAKING):
  - the embedded `ConnectorNode`, parent BTP peering, ILP client, connector
    admin/channel clients, and the `POST /handle-packet` route;
  - the x402 `/publish` flow (preflight, pricing, settlement, EIP-3009);
  - chain/settlement resolution and peer/seed discovery (kind:10032);
  - the `BusinessLogicServer` and pricing modules and their exports;
  - the deprecated `startTown` / `Town*` launcher aliases;
  - the dependencies `@toon-protocol/connector`, `@toon-protocol/sdk`, `viem`,
    `@toon-protocol/core`, and the `@toon-protocol/bls` workspace dependency.

  The relay used `@toon-protocol/core` only for the TOON event codec, which
  transitively pulled the Arweave / web3 wallet stack; that ~120-line codec is
  now vendored (`src/toon/codec.ts`, depends only on `@toon-format/toon`).

  There is no longer a separate "oblivious" mode — it is the only behavior, so
  the flag and the `oblivious` naming are gone. Identity is derived with
  `nostr-tools` (NIP-06 for mnemonics); `NOSTR_SECRET_KEY` is honored as an alias
  for `TOON_SECRET_KEY`. Ships as the `ghcr.io/toon-protocol/relay:latest` image.

### Patch Changes

- b8ec120: fix(deps): bump @toon-protocol/core to ^1.4.2 and @toon-protocol/sdk to ^0.5.1

  Unblocks CI. The previously-pinned `@toon-protocol/core@1.4.1` tarball was
  re-published in place on npm (lockfile integrity no longer matched), which
  forced pnpm to re-resolve and then fail on `@toon-protocol/sdk@0.5.0`'s leaked
  `@toon-protocol/core@workspace:*` dependency. `core@1.4.2`/`sdk@0.5.1` have clean
  integrity and `sdk@0.5.1` resolves core to a concrete `1.4.2`, so the lockfile is
  regenerated against trustworthy tarballs.

## 1.3.4

### Patch Changes

- 591fe07: fix(deps): bump @toon-protocol/core to ^1.4.2 and @toon-protocol/sdk to ^0.5.1

  Unblocks CI. The previously-pinned `@toon-protocol/core@1.4.1` tarball was
  re-published in place on npm (lockfile integrity no longer matched), which
  forced pnpm to re-resolve and then fail on `@toon-protocol/sdk@0.5.0`'s leaked
  `@toon-protocol/core@workspace:*` dependency. `core@1.4.2`/`sdk@0.5.1` have clean
  integrity and `sdk@0.5.1` resolves core to a concrete `1.4.2`, so the lockfile is
  regenerated against trustworthy tarballs.

- Updated dependencies [591fe07]
  - @toon-protocol/bls@1.2.4

## 1.3.3

### Patch Changes

- a5c2d90: fix(deps): bump @toon-protocol/core to ^1.4.2 and @toon-protocol/sdk to ^0.5.1

  Unblocks CI. The previously-pinned `@toon-protocol/core@1.4.1` tarball was
  re-published in place on npm (lockfile integrity no longer matched), which
  forced pnpm to re-resolve and then fail on `@toon-protocol/sdk@0.5.0`'s leaked
  `@toon-protocol/core@workspace:*` dependency. `core@1.4.2`/`sdk@0.5.1` have clean
  integrity and `sdk@0.5.1` resolves core to a concrete `1.4.2`, so the lockfile is
  regenerated against trustworthy tarballs.

- Updated dependencies [a5c2d90]
  - @toon-protocol/bls@1.2.3
