---
'@toon-protocol/relay': minor
---

The Relay Information Document lists the connector's real settlement terms.

The connector replaced `settlements` on `GET /ilp` with `batchSettlements`, so
the relay's `toon.settlement` was `[]` on devnet. It now reads
`batchSettlements` and carries one `{network, asset}` entry per term, values
copied verbatim. The retired `settlements` key is no longer read, and a
self-description without `batchSettlements` yields `[]` without stopping the
Write Edge being published.

**Breaking shape change:** entries were `{chain, token, decimals}` and are now
`{network, asset}`. This is knowingly ahead of TOON Network spec §13.1, which
still specifies the old shape; the connector publishes no `decimals`.
