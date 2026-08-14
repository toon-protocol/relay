# @toon-protocol/relay

A Nostr relay app: free NIP-01 WebSocket reads, an HTTP `POST /write` surface
for storing events, and a free `POST /write-ephemeral` lane for
presence/typing traffic (relay#129).

The relay contains **no ILP, connector, settlement, or pricing logic**. Payment
is enforced entirely upstream by an external terminator — by the time a write
reaches this process it is already proven paid, so the relay simply stores the
event and serves reads.

## Install

```bash
npm install @toon-protocol/relay
```

## Run (CLI)

```bash
NOSTR_SECRET_KEY=<64-char-hex> npx @toon-protocol/relay
# reads:      ws://localhost:7100
# writes:     http://localhost:3100/write
# ephemeral:  http://localhost:3100/write-ephemeral (free, relay#129)
# health:     http://localhost:3100/health
```

| Env var                                | Default   | Description                                                                                                                       |
| -------------------------------------- | --------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `TOON_SECRET_KEY` / `NOSTR_SECRET_KEY` | —         | 64-char hex identity key (one of these or `TOON_MNEMONIC` is required)                                                            |
| `TOON_MNEMONIC`                        | —         | BIP-39 mnemonic (NIP-06 derivation)                                                                                               |
| `TOON_RELAY_PORT`                      | `7100`    | WebSocket read port                                                                                                               |
| `TOON_BLS_PORT`                        | `3100`    | HTTP write/health port                                                                                                            |
| `TOON_HOST`                            | `0.0.0.0` | WebSocket bind host                                                                                                               |
| `TOON_WRITE_HOST`                      | `0.0.0.0` | HTTP write/health bind host (see [write-port exposure](#paid-ephemeral-verify-skip-relay85))                                      |
| `TOON_DATA_DIR`                        | `./data`  | SQLite data directory                                                                                                             |
| `TOON_DEV_MODE`                        | `false`   | Skip event-signature verification on `POST /write`                                                                                |
| `TOON_VERIFY_EPHEMERAL`                | `false`   | Run FULL schnorr verification on ephemeral kinds too (see below)                                                                  |
| `TOON_VERIFY_WORKERS`                  | CPUs − 1  | Worker threads for persistent-kind signature verification; `0` = inline on the event loop (automatic on 1-core boxes)             |
| `TOON_MAX_CONNECTIONS`                 | `4096`    | Maximum concurrent WebSocket read connections (each costs one file descriptor — mind `ulimit -n`)                                 |
| `TOON_EPHEMERAL_RATE_LIMIT`            | `200`     | Free ephemeral lane (`POST /write-ephemeral`): max requests per key per window — see [below](#free-ephemeral-write-lane-relay129) |
| `TOON_EPHEMERAL_RATE_WINDOW_MS`        | `10000`   | Free ephemeral lane: rate-limit window in milliseconds                                                                            |
| `TOON_EPHEMERAL_MAX_BODY_BYTES`        | `8192`    | Free ephemeral lane: request body size cap in bytes                                                                               |

## Paid-ephemeral verify skip (relay#85)

By default the relay **skips schnorr verification for ephemeral kinds**
(NIP-16, `20000 <= kind < 30000`) on `POST /write` and keeps only the SHA-256
event-id check. This is a deliberate, payment-gated bypass:

- **Why it is safe here:** every request reaching `POST /write` has already
  passed the upstream connector's payment claim gate — payment is the
  admission/spam gate. Protocol rule: clients trust the signature chain and
  verify every event themselves, never the relay. Relay-side schnorr on paid
  ephemeral frames buys no additional trust; forging a speaker costs real
  money to emit frames every client discards.
- **What is still checked:** the SHA-256 id check always runs, so the relay
  never broadcasts bytes that disagree with the event id clients verify by.
- **When you MUST turn it off:** if your write port is fronted by anything
  other than a payment-gating connector — or you ever add a FREE ephemeral
  write lane — set `TOON_VERIFY_EPHEMERAL=true` (`--verify-ephemeral`,
  `verifyEphemeral: true`). A free lane must NOT reuse this skip.
- **Exposure guard:** the write port must be reachable only via the
  connector. In docker, never host-publish it (`expose:`, not `ports:` —
  docker-published ports bypass ufw). Outside docker, bind it internally via
  `TOON_WRITE_HOST=127.0.0.1`. At startup the relay logs a prominent warning
  if the write listener binds a non-internal interface while the skip is
  active (warning only — container topologies legitimately bind `0.0.0.0`
  and stay private by not publishing the port).

## Free ephemeral write lane (relay#129)

`POST /write-ephemeral` is a SECOND write surface, distinct from the paid
`POST /write` above, because the connector cannot carry two prices on one
`handler_url` (`ConflictingHandlerPrice`) — a free lane needs its own
endpoint, terminated by its own `price = 0` route
(`deploy/connector.toml`'s `g.toon.relay.ephemeral`).

- **Ephemeral kinds only** (NIP-16, `20000 <= kind < 30000`) — anything else
  is a `400`. A persistent kind on this lane would be a free ride around
  pay-to-write.
- **Never stores.** Ephemeral kinds are never persisted on the paid path
  either (NIP-16); this lane broadcasts to live WS subscribers and nothing
  else.
- **Full schnorr verification, always.** Unlike the paid handler's
  payment-gated ephemeral verify skip, this lane has no payment admission
  gate — signature verification IS its only defense against forged-signature
  spam, so there is no config knob to skip it here.
- **Bounded**, because free + broadcast = spam surface: a per-key sliding-
  window rate limit and a request-body size cap, both config-gated with
  conservative defaults (`TOON_EPHEMERAL_RATE_LIMIT` /
  `TOON_EPHEMERAL_RATE_WINDOW_MS` / `TOON_EPHEMERAL_MAX_BODY_BYTES` above).
  Both bounds are logged at startup and surfaced on `GET /metrics`
  (`ephemeralWriteLane`).

## Run (programmatic)

```ts
import { startRelay } from '@toon-protocol/relay';

const relay = await startRelay({ secretKey });
// ... POST /write on 3100, read NIP-01 on 7100 ...
await relay.stop();
```

## HTTP surface

| Method | Path               | Description                                                                                                                                                                                                                                                                                                                             |
| ------ | ------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST` | `/write`           | Store an event. Body `{ "event": <NostrEvent> }`. No payment headers are read or echoed (the terminating connector asserts nothing about payment to this relay); verifies only the event signature (ephemeral kinds: id check only by default, see above).                                                                              |
| `POST` | `/write-ephemeral` | Free ephemeral write lane (relay#129, see above). Body `{ "event": <NostrEvent> }`. Ephemeral kinds only (`400` otherwise); always full schnorr verification; never stores; rate-limited and size-capped (`429` / `413`).                                                                                                               |
| `GET`  | `/health`          | Liveness, identity (`pubkey`), `capabilities`, and `version`.                                                                                                                                                                                                                                                                           |
| `GET`  | `/metrics`         | JSON telemetry: `eventLoopDelayMs` (mean/p50/p99/max — loop lag is ephemeral-frame tail latency), `verify` (per-event verify wall time incl. pool queueing, active implementation, worker count — the trigger metrics for scaling decisions, relay#85), and `ephemeralWriteLane` (enablement + rate-limit/body-size bounds, relay#129). |

## WebSocket Relay Server

NIP-01 compliant WebSocket server that stores and serves Nostr events in TOON format.

```ts
import { NostrRelayServer, SqliteEventStore } from '@toon-protocol/relay';

const eventStore = new SqliteEventStore('./events.db');
const relay = new NostrRelayServer({ port: 7100 }, eventStore);

await relay.start();
relay.broadcastEvent(event); // push to matching subscriptions
await relay.stop();
```

## Event Storage

```ts
import { InMemoryEventStore, SqliteEventStore } from '@toon-protocol/relay';

const memStore = new InMemoryEventStore(); // ephemeral
const sqlStore = new SqliteEventStore('./events.db'); // persistent

memStore.store(event);
const found = memStore.get(event.id);
const results = memStore.query([{ kinds: [1], limit: 10 }]);
```

## TOON Codec

Vendored in-repo (`src/toon/codec.ts`) so the relay depends only on the lightweight `@toon-format/toon` encoder rather than `@toon-protocol/core`'s full transitive tree. The relay has no runtime dependency on `@toon-protocol/core`.

```ts
import { encodeEventToToon, decodeEventFromToon } from '@toon-protocol/relay';
```

## Full API

| Category          | Exports                                                                                  |
| ----------------- | ---------------------------------------------------------------------------------------- |
| **Launcher**      | `startRelay`, `RelayConfig`, `RelayInstance`, `RelaySubscription`, `ResolvedRelayConfig` |
| **Relay**         | `NostrRelayServer`, `ConnectionHandler`, `RelayServerConfig`, `DEFAULT_RELAY_CONFIG`     |
| **Storage**       | `EventStore`, `InMemoryEventStore`, `SqliteEventStore`, `RelayError`                     |
| **Write/Health**  | `createWriteHandler`, `createEphemeralWriteHandler`, `createHealthResponse`              |
| **Rate limiting** | `createRateLimiter` (backs the ephemeral write lane's bounds, relay#129)                 |
| **Codec**         | `encodeEventToToon`, `decodeEventFromToon`, `ToonEncodeError`, `ToonDecodeError`         |
| **Subscriber**    | `RelaySubscriber`, `RelaySubscriberConfig`                                               |
| **Filter**        | `matchFilter`                                                                            |
| **Constants**     | `VERSION`                                                                                |

## License

MIT
