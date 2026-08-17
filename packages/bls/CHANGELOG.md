# @toon-protocol/bls

## 1.3.0

### Minor Changes

- d565a88: The genesis-peer self-announce now carries a NIP-40 `["expiration", created_at + ttl]` tag and is republished on a refresh loop, instead of being written into this node's own event store exactly once at boot with no expiry at all.

  This relay refuses to _serve_ an expired announce as of the previous release. That fix has a hole for as long as any publisher still emits announces that never expire — and `packages/bls/src/entrypoint.ts` was one of them, writing straight into the store it serves from. kind:10032 is a **replaceable** event: a relay keeps the latest one per author, and the only retraction path is a newer event signed by the same key. Once that key is gone the advertisement can be neither replaced nor NIP-09-deleted, so the litter is permanent by construction and clients keep dialing a dead BTP endpoint. Devnet is carrying exactly that today: `b23599a6…` / `g.toon.swap.sol`, key gone, advertising a `ws://127.0.0.1:3401` loopback literal that resolves to whatever machine reads it.

  The expiry and the refresh loop are inseparable. This publisher had **no refresh loop of any kind**, so an expiry on its own would have taken a live genesis peer out of its own discovery one TTL after start-up — worse than the litter it removes. The new `packages/bls/src/announce.ts` owns both halves, and re-signs each round rather than re-storing a cached event (the tag is `created_at + ttl`, so a cached event's expiry recedes into the past however often it is republished).

  Two new **optional** environment variables, both defaulting to the fleet convention: `ANNOUNCE_TTL_SECONDS` (600, the Rust connector's `[announce] ttl_secs` default) and `ANNOUNCE_REFRESH_SECONDS` (240, every `connector announce` loop overlay's `REFRESH_SECS`) — the same ~6 minutes of continuous headroom that was measured live for the serve-time enforcement. Neither can fail boot: an unusable value falls back to its default with a warning rather than throwing, because every service on this fleet auto-deploys on green main and a rejected value would be a crash loop on a live box, not a build failure. `0` on either is a documented escape hatch (never expire / never refresh) and says so loudly; a refresh interval that does not beat the TTL is reported at `error`, that being the one misconfiguration worse than the litter.

### Patch Changes

- Updated dependencies [3eec8b3]
  - @toon-protocol/relay@2.1.0

## 1.2.5

### Patch Changes

- b8ec120: fix(deps): bump @toon-protocol/core to ^1.4.2 and @toon-protocol/sdk to ^0.5.1

  Unblocks CI. The previously-pinned `@toon-protocol/core@1.4.1` tarball was
  re-published in place on npm (lockfile integrity no longer matched), which
  forced pnpm to re-resolve and then fail on `@toon-protocol/sdk@0.5.0`'s leaked
  `@toon-protocol/core@workspace:*` dependency. `core@1.4.2`/`sdk@0.5.1` have clean
  integrity and `sdk@0.5.1` resolves core to a concrete `1.4.2`, so the lockfile is
  regenerated against trustworthy tarballs.

- Updated dependencies [b8ec120]
- Updated dependencies [b8ec120]
  - @toon-protocol/relay@2.0.0

## 1.2.4

### Patch Changes

- 591fe07: fix(deps): bump @toon-protocol/core to ^1.4.2 and @toon-protocol/sdk to ^0.5.1

  Unblocks CI. The previously-pinned `@toon-protocol/core@1.4.1` tarball was
  re-published in place on npm (lockfile integrity no longer matched), which
  forced pnpm to re-resolve and then fail on `@toon-protocol/sdk@0.5.0`'s leaked
  `@toon-protocol/core@workspace:*` dependency. `core@1.4.2`/`sdk@0.5.1` have clean
  integrity and `sdk@0.5.1` resolves core to a concrete `1.4.2`, so the lockfile is
  regenerated against trustworthy tarballs.

- Updated dependencies [591fe07]
  - @toon-protocol/relay@1.3.4

## 1.2.3

### Patch Changes

- a5c2d90: fix(deps): bump @toon-protocol/core to ^1.4.2 and @toon-protocol/sdk to ^0.5.1

  Unblocks CI. The previously-pinned `@toon-protocol/core@1.4.1` tarball was
  re-published in place on npm (lockfile integrity no longer matched), which
  forced pnpm to re-resolve and then fail on `@toon-protocol/sdk@0.5.0`'s leaked
  `@toon-protocol/core@workspace:*` dependency. `core@1.4.2`/`sdk@0.5.1` have clean
  integrity and `sdk@0.5.1` resolves core to a concrete `1.4.2`, so the lockfile is
  regenerated against trustworthy tarballs.

- Updated dependencies [a5c2d90]
  - @toon-protocol/relay@1.3.3
