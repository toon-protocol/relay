# @toon-protocol/relay

A Nostr relay app: free NIP-01 WebSocket reads, an HTTP `POST /write` surface
for storing events, and a free `POST /write-ephemeral` lane for presence and
typing traffic.

The relay contains **no payment, settlement, or ILP logic**. Payment is
enforced upstream by a terminating connector; by the time a write reaches this
process it is already paid for, so the relay verifies the event signature,
stores it, and serves reads. To run one behind a payment proxy, see the
[repository README](https://github.com/toon-protocol/relay#readme).

## Install

```bash
npm install @toon-protocol/relay
```

## Run

```bash
NOSTR_SECRET_KEY=<64-char-hex> npx @toon-protocol/relay
# reads:      ws://localhost:7100
# writes:     http://localhost:3100/write
# ephemeral:  http://localhost:3100/write-ephemeral
# health:     http://localhost:3100/health
```

An identity key is required: one of `--secret-key` / `TOON_SECRET_KEY` /
`NOSTR_SECRET_KEY` (64 hex characters), or `--mnemonic` / `TOON_MNEMONIC`
(BIP-39, NIP-06 derivation). Prefer the environment variables — a secret
passed as a flag is visible in process listings, and the CLI warns when you do
it.

| Flag                                 | Environment variable                    | Default   | Description                                                                 |
| ------------------------------------ | --------------------------------------- | --------- | --------------------------------------------------------------------------- |
| `--secret-key`                       | `TOON_SECRET_KEY`, `NOSTR_SECRET_KEY`   | —         | 64-hex identity key                                                         |
| `--mnemonic`                         | `TOON_MNEMONIC`                         | —         | BIP-39 mnemonic (NIP-06)                                                    |
| `--relay-port`                       | `TOON_RELAY_PORT`                       | `7100`    | WebSocket read port                                                         |
| `--bls-port`                         | `TOON_BLS_PORT`                         | `3100`    | HTTP write / health / metrics port                                          |
| `--host`                             | `TOON_HOST`                             | `0.0.0.0` | read-port bind address                                                      |
| `--write-host`                       | `TOON_WRITE_HOST`                       | `0.0.0.0` | write-port bind address (see [exposure](#the-write-port-must-stay-private)) |
| `--data-dir`                         | `TOON_DATA_DIR`                         | `./data`  | SQLite directory                                                            |
| `--dev-mode`                         | `TOON_DEV_MODE`                         | `false`   | skip signature verification entirely — smoke tests only                     |
| `--verify-ephemeral`                 | `TOON_VERIFY_EPHEMERAL`                 | `false`   | full verification on paid ephemeral kinds too                               |
| `--verify-workers`                   | `TOON_VERIFY_WORKERS`                   | CPUs − 1  | verify-pool threads; `0` verifies on the event loop                         |
| `--max-connections`                  | `TOON_MAX_CONNECTIONS`                  | `4096`    | concurrent WS reads (one file descriptor each)                              |
| `--log-writes`                       | `TOON_LOG_WRITES`                       | `false`   | one log line per accepted write                                             |
| `--no-enforce-expiration`            | `TOON_ENFORCE_EXPIRATION`               | enforced  | serve events past their NIP-40 `expiration` again                           |
| `--expiration-reap-grace-seconds`    | `TOON_EXPIRATION_REAP_GRACE_SECONDS`    | `86400`   | how long an expired event stays on disk                                     |
| `--expiration-reap-interval-seconds` | `TOON_EXPIRATION_REAP_INTERVAL_SECONDS` | `3600`    | how often the reaper sweeps; `0` never                                      |
| `--blocked-event-ids`                | `TOON_BLOCKED_EVENT_IDS`                | —         | comma-separated 64-hex event ids to refuse                                  |
| `--ephemeral-rate-limit`             | `TOON_EPHEMERAL_RATE_LIMIT`             | `200`     | free-lane requests per key per window                                       |
| `--ephemeral-rate-window-ms`         | `TOON_EPHEMERAL_RATE_WINDOW_MS`         | `10000`   | free-lane rate-limit window                                                 |
| `--ephemeral-max-body-bytes`         | `TOON_EPHEMERAL_MAX_BODY_BYTES`         | `8192`    | free-lane request body cap                                                  |

Retention behaviour — NIP-40 expiry, NIP-09 deletion, the blocklist — is
documented in
[`docs/retention.md`](https://github.com/toon-protocol/relay/blob/main/docs/retention.md).

## HTTP surface

| Method | Path               | Description                                                                                                                                                                                    |
| ------ | ------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST` | `/write`           | Store an event. Body `{ "event": <NostrEvent> }`. Verifies the signature (ephemeral kinds: id check only, by default). Echoes the connector's payment statement when there is one — see below. |
| `POST` | `/write-ephemeral` | The free ephemeral lane. Ephemeral kinds only (`400` otherwise), always fully verified, never stored, rate-limited and size-capped (`429` / `413`).                                            |
| `GET`  | `/health`          | Liveness, identity (`pubkey`), `capabilities`, `version`.                                                                                                                                      |
| `GET`  | `/metrics`         | JSON telemetry: event-loop delay, per-event verify timings and pool state, and the ephemeral lane's bounds.                                                                                    |

### The payment statement

A terminating connector states three headers on a delivery whose payment it
verified at its own client edge:

| Header          | Value                                                          |
| --------------- | -------------------------------------------------------------- |
| `X-TOON-Payer`  | the client channel key — `evm:0x<64 hex>` or `solana:<base58>` |
| `X-TOON-Amount` | the route's price, decimal, in base units                      |
| `X-TOON-Chain`  | `evm` or `solana`                                              |

When all three are present and well-formed, `POST /write` returns them as a
`payment` object and includes them on its log line. The relay re-validates
none of it — it holds no chain state — but a malformed or partial statement is
discarded whole rather than half-recorded.

**Absence is not "unpaid".** The headers are stated only by the hop that took
the payment, so they are absent on a forwarded packet and on every free route.
Never treat their absence as a reason to refuse a write.

## The write port must stay private

By default the relay **skips signature verification for ephemeral kinds**
(NIP-16, `20000 <= kind < 30000`) on `POST /write`, keeping only the SHA-256
id check. That is a deliberate, payment-gated bypass:

- **Why it is safe:** every request reaching `POST /write` has already passed
  the connector's payment gate — payment is the admission control. Clients
  verify every signature themselves and never trust the relay's verdict, so
  relay-side verification of paid ephemeral frames buys no trust that payment
  has not already bought.
- **What is still checked:** the id check always runs, so the relay never
  broadcasts bytes that disagree with the id clients index by.
- **When to turn it off:** if the write port is fronted by anything other than
  a payment-gating connector, set `TOON_VERIFY_EPHEMERAL=true`.

In Docker, never host-publish the write port (`expose:`, not `ports:` — a
docker publish bypasses ufw). Outside Docker, bind it with
`TOON_WRITE_HOST=127.0.0.1`. The relay logs a warning at startup if the write
listener binds a non-internal interface while the skip is active.

## The free ephemeral lane

`POST /write-ephemeral` is a second write surface, distinct from the paid one,
because a connector cannot carry two prices on a single handler URL. It is
terminated by its own zero-priced route.

It accepts **only** ephemeral kinds, **never** stores, and **always** verifies
signatures in full — it has no payment gate to lean on, so verification is its
only defence against forged-signature spam. It is bounded by a per-key sliding
window and a body-size cap, both surfaced on `GET /metrics`.

## Programmatic use

```ts
import { startRelay } from '@toon-protocol/relay';

const relay = await startRelay({ secretKey });
// POST /write on 3100, NIP-01 reads on 7100
await relay.stop();
```

```ts
import { NostrRelayServer, SqliteEventStore } from '@toon-protocol/relay';

const eventStore = new SqliteEventStore('./events.db');
const relay = new NostrRelayServer({ port: 7100 }, eventStore);

await relay.start();
relay.broadcastEvent(event); // push to matching subscriptions
await relay.stop();
```

| Category              | Exports                                                                                  |
| --------------------- | ---------------------------------------------------------------------------------------- |
| **Launcher**          | `startRelay`, `RelayConfig`, `RelayInstance`, `RelaySubscription`, `ResolvedRelayConfig` |
| **Relay**             | `NostrRelayServer`, `ConnectionHandler`, `RelayServerConfig`, `DEFAULT_RELAY_CONFIG`     |
| **Storage**           | `EventStore`, `InMemoryEventStore`, `SqliteEventStore`, `RelayError`                     |
| **Write / health**    | `createWriteHandler`, `createEphemeralWriteHandler`, `createHealthResponse`              |
| **Payment statement** | `readPaymentAttribution`, `PaymentAttribution`                                           |
| **Rate limiting**     | `createRateLimiter`                                                                      |
| **Subscriber**        | `RelaySubscriber`, `RelaySubscriberConfig`                                               |
| **Filter**            | `matchFilter`                                                                            |
| **Constants**         | `VERSION`                                                                                |

## License

MIT
