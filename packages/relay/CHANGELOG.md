# @toon-protocol/relay

## 2.1.0

### Minor Changes

- 3eec8b3: Enforce NIP-40 expiration, implement NIP-09 deletion, and add an operator event blocklist.

  Until now the relay had no way to stop serving an event. Expiry tags were parsed by nothing, kind:5 was stored as an ordinary note, and the only retraction path was a newer event signed by the same key — so a node whose key was lost advertised itself forever.
  - **NIP-40**: events past their `expiration` tag are no longer served from history or fanned out live, and a background reaper deletes them after a grace period. On by default; `TOON_ENFORCE_EXPIRATION=false` is the kill switch back to the old behaviour.
  - **NIP-09**: a kind:5 request retracts the author's own events by `e` id or `a` coordinate, and a tombstone stops a re-publish from resurrecting them. A request naming another key's event is a no-op.
  - **Operator blocklist** (`TOON_BLOCKED_EVENT_IDS`): startup-configured event ids this relay refuses to store or serve — the escape hatch for litter whose author key is gone, so neither NIP can reach it. Ids only, never pubkeys; logged loudly on every boot.

  Existing SQLite databases are migrated in place (an `expires_at` column, backfilled from stored tags), so a pre-existing expired event stops being served on the first boot after upgrading with no republish required.

- f616690: Record the payment the connector states on a delivery, and make `deploy/` the deployment of record.

  `POST /write` reads `X-TOON-Payer` / `X-TOON-Amount` / `X-TOON-Chain` again (connector ADR 0040, relay#133). #128 removed them on the strength of ADR 0036's "no successor header is coming"; one came, and it is a different value: the payer is now the chain-verified client channel key whose claim the terminating connector checked at its own edge, not the previous hop. A well-formed triple is echoed on the 200 as `payment` and carried on the write's log line; a partial or malformed one is discarded whole rather than half-recorded, and **absence is never read as "unpaid"** — the headers are stated only by the hop that took the payment, so they are absent on a forwarded packet and on every free route.

  `deploy/` now runs the connector this contract comes from (`rust-sha-6ea6009`, up from `rust-sha-440eab7`) and carries the whole node rather than half of it: Caddy terminates TLS for both public hostnames, a Watchtower overlay follows the two `:release` tags, and a local overlay runs the stack without TLS. The connector config drops `[announce]` — the current connector refuses that section by name and serves the same facts on `GET /ilp` from a `[node]` table instead.

  Also: `packages/bls` is removed (nothing depended on it, no CI built its images, and it was the only ILP-speaking code in a repo that documents itself as speaking none), the unused TOON codec re-export is gone, `DEFAULT_RELAY_CONFIG.port` now matches the launcher's actual default of 7100, and `GET /health` reports the version the package actually shipped instead of a hardcoded `0.1.0`.

### Patch Changes

- dd5c8a9: Take `VERSION` from `package.json` at build time instead of hand-copying it.

  The constant behind `GET /health` was written down in `src/version.ts`, and `changeset version` bumps `package.json` and nothing else — so the two parted company the moment a release was cut. That is how `/health` came to serve a hardcoded `0.1.0` from a shipped `2.0.2`. The guard added alongside that fix compared the two, which turned the drift into a red Release PR rather than removing it: every release now failed CI until someone edited the source by hand.

  `src/version.ts` now carries a `__RELAY_VERSION__` placeholder that tsup substitutes from `package.json` when it builds the bundle, and that both vitest configs substitute the same way when they run the tests. `package.json` is the only place a version is written, a release needs no source edit, and the shipped bundle reports the version it was cut from.

## 2.0.2

### Patch Changes

- 6ed12ab: Lift the paid-write pipeline's ~150 events/s global admission ceiling (connector#685): ephemeral events (NIP-16, kinds 20000-29999) are now broadcast-only and never hit the disk, the SQLite event store runs in WAL mode with synchronous=NORMAL instead of the default rollback journal with two fsyncs per insert, and the regular-event INSERT statement is prepared once instead of on every write. The synchronous per-event disk write on the single Node event loop was the serialization point that capped aggregate paid-write throughput at ~150 frames/s across all BTP sessions while CPU stayed idle.
- dd881d9: Lift the post-#84 write-path CPU ceiling (relay#85, part 1 of the ticket): `POST /write` now verifies BIP-340 event signatures with WASM libsecp256k1 (`tiny-secp256k1`, ~0.20ms/verify) instead of pure-JS noble (~1.3ms/verify) — about 7x faster per event — with a load-time self-test and a transparent fallback to nostr-tools' noble verify if the WASM module cannot load, so the relay never hard-fails on an unsupported platform. The active implementation is logged once at startup. Additionally, the per-write `[write] event=...` console line is now off by default and opt-in via `--log-writes` / `TOON_LOG_WRITES=true`: per-event console I/O through docker's json-file log driver was residual write-path disk I/O and event-loop tail jitter that #84 did not remove (connector#685 Phase G).
- 92678d2: Skip schnorr verification for paid ephemeral kinds on `POST /write` (relay#85, decision 2026-08-02): events with `20000 <= kind < 30000` now pass only the SHA-256 event-id check by default — the BIP-340 signature check is skipped. This is safe ONLY because the write path is payment-gated (the upstream connector's claim gate is the admission gate) and clients verify every signature themselves; a future FREE ephemeral lane must NOT reuse this bypass. Full verification can be restored with `--verify-ephemeral` / `TOON_VERIFY_EPHEMERAL=true` / `verifyEphemeral: true` (for operators fronting the write port with anything other than a payment-gating connector). Also adds the write-port exposure guard: a new `--write-host` / `TOON_WRITE_HOST` bind option for the HTTP write/health listener, plus a prominent startup warning (never a hard failure) when the write listener binds a non-loopback/non-internal interface while the skip is active. The canonical compose deploy already keeps `:3100` unpublished (`expose:`, not `ports:` — docker-published ports bypass ufw) and now documents that this is a security precondition, not just privacy.
- 05f550e: Worker-thread verify pool + `GET /metrics` telemetry (relay#85). Persistent-kind signature verification on `POST /write` now runs on a hand-rolled `worker_threads` pool (default size `max(0, os.cpus().length - 1)`; `--verify-workers` / `TOON_VERIFY_WORKERS`; `0` = the previous inline path, automatic on 1-core boxes). Each worker loads its own WASM libsecp256k1 with the same self-test + noble-fallback semantics; worker failure degrades transparently to inline verification. Rationale: agent writers make persistent-write rates bursty — the pool keeps verify bursts off the event loop so they cannot jitter ephemeral (huddle-frame) latency. Per-session write ordering is unaffected: the upstream connector serializes each BTP session's POSTs, and a test pins that sequentially awaited writes never reorder. New `GET /metrics` on the write/health port exposes the scaling trigger metrics: event-loop delay (mean/p50/p99/max ms) and per-event verify time (count/mean/p50/p99/max ms, including pool queueing) plus the active verify implementation and worker count.
- d80f279: Two fan-out fixes from the 2026-08-02 benchmarking (relay#90, relay#91). (1) `maxConnections` default raised 100 → 4096 and made clearly configurable (`--max-connections` / `TOON_MAX_CONNECTIONS` / `maxConnections` through the launcher): the stock 100 made more than 100 huddle listeners impossible; connections are fd-shaped (one descriptor + a few KB each), and 4096 supports several hundred-listener huddles while sitting far below docker's default nofile limit (1048576) — an advisory startup warning fires if the cap exceeds the process's soft fd limit, and rejected connections now log. (2) Serialize-once broadcast: the ephemeral fan-out loop stringified the identical event once PER subscriber (500 subscribers = 500 identical serializations per frame, measured pinning a core); `broadcastEvent` now serializes the event payload once and splices the per-subscription NIP-01 envelope (`serializeEventFrame`, pinned byte-identical to the full `JSON.stringify(['EVENT', subId, event])`).

## 2.0.1

### Patch Changes

- add0c47: Serve canonical NIP-01 JSON on outbound EVENT frames (#46). The WebSocket read surface previously encoded the event as a TOON-text string inside the frame (`["EVENT", subId, "<toon text>"]`), which standard nostr clients could neither parse nor signature-verify from the wire. Both the stored-query (REQ) and live-subscription (broadcast) paths now emit `["EVENT", subId, {id, pubkey, created_at, kind, tags, content, sig}]` with the event as a plain JSON object, byte-compatible with vanilla NIP-01 libraries. The TOON codec remains exported for library consumers but is no longer used on the wire.

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
