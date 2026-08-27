---
'@toon-protocol/relay': patch
---

Take `VERSION` from `package.json` at build time instead of hand-copying it.

The constant behind `GET /health` was written down in `src/version.ts`, and `changeset version` bumps `package.json` and nothing else — so the two parted company the moment a release was cut. That is how `/health` came to serve a hardcoded `0.1.0` from a shipped `2.0.2`. The guard added alongside that fix compared the two, which turned the drift into a red Release PR rather than removing it: every release now failed CI until someone edited the source by hand.

`src/version.ts` now carries a `__RELAY_VERSION__` placeholder that tsup substitutes from `package.json` when it builds the bundle, and that both vitest configs substitute the same way when they run the tests. `package.json` is the only place a version is written, a release needs no source edit, and the shipped bundle reports the version it was cut from.
