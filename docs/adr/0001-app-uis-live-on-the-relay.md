# App UIs live on the relay

**Status:** Accepted (owner decision, 2026-10-22, #276). Recorded in #277; the UIs themselves are built by the sub-issues of #276, so nothing in this record is built yet.

**Scope:** relay architecture — internal to this codebase. See the [ADR index](README.md).

**Falsifier:** a UI crate that holds a write handle to the store, or the relay making an outbound HTTP request to produce a UI page, or a plain `GET /` that no longer returns `426`.

**The relay serves a read-only UI for each Nostr app type on its read port, from optional crates mounted under fixed path prefixes, and it never fetches anything to render them.** It still speaks no ILP: writes remain paid packets that reach `POST /write` through the connector.

## Context

NIP-34 (git over Nostr) repos on TOON are read through rig-web, a React/Vite app published to Arweave and owned by a CLI that is being sunset. Anything Nostr is to live in this repo, and each Nostr app type should get a UI at its own endpoint (`/repo` for NIP-34, `/social` later). The relay's read port already answers WebSocket upgrades and NIP-11 on any path, and fleet health checks rely on a plain `GET /` returning `426`.

## Decision

1. **A UI is an optional crate under a fixed read-port prefix.** A `relay-ui` crate defines what an app UI is: a mount prefix plus an axum `Router`. Each app is its own crate (`ui-nip34` first), behind a Cargo feature of the `relay` binary, and an operator switches apps on or off at runtime.
2. **A UI is built from a read-only in-process query handle.** The handle is a narrow trait over `Store::query(Filter)` and offers no write. UIs read this relay's store directly, so they only show this relay's events, and they never open a loopback WebSocket. There are no browser writes.
3. **The relay never makes an outbound fetch for UI content.** Git objects and blobs are fetched by the **visitor's browser**, from URLs named in the repo's own events (the announcement's `blossoms` tag, `arweave` tags on 30618) and then from an optional operator default list. The server-rendered page lists the candidates; the browser checks each object's hash. The relay is therefore never an open proxy and takes no SSRF exposure from event content.
4. **Every path outside the UI prefixes keeps today's behaviour.** Only `GET`/`HEAD` under a prefix (and its assets) is new. WebSocket upgrade and NIP-11 on any path are unchanged, and a plain `GET /` still returns `426`.

## Consequences

- A relay built without the UI features, or run with them switched off, is byte-for-byte today's relay on the read port.
- A missing, expired or mismatched object is a normal page state, not a server error, because the relay cannot repair it.
- The browser needs `connect-src https:` on git pages; pages built only from events stay server-rendered with a strict CSP.
- A feature that needs the relay to fetch on a visitor's behalf is a new decision and needs its own record.
