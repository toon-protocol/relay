---
"@toon-protocol/relay": major
---

refactor(relay)!: make the relay a pure HTTP/WebSocket app — remove all ILP/connector logic and dependencies

The relay no longer speaks ILP or embeds a connector. Payment is enforced
entirely upstream by an external terminator, so the relay is now just:

- free NIP-01 WebSocket reads (`TOON_RELAY_PORT`, default 7100), and
- an HTTP `POST /write` surface plus `GET /health` (`TOON_BLS_PORT`, default
  3100) that trusts injected `X-TOON-*` headers without re-validating payment.

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
