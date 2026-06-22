# @toon-protocol/relay

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
