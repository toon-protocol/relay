# @toon-protocol/relay

A Nostr relay app: free NIP-01 WebSocket reads plus an HTTP `POST /write`
surface for storing events.

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
# reads:  ws://localhost:7100
# writes: http://localhost:3100/write
# health: http://localhost:3100/health
```

| Env var | Default | Description |
|---------|---------|-------------|
| `TOON_SECRET_KEY` / `NOSTR_SECRET_KEY` | — | 64-char hex identity key (one of these or `TOON_MNEMONIC` is required) |
| `TOON_MNEMONIC` | — | BIP-39 mnemonic (NIP-06 derivation) |
| `TOON_RELAY_PORT` | `7100` | WebSocket read port |
| `TOON_BLS_PORT` | `3100` | HTTP write/health port |
| `TOON_DATA_DIR` | `./data` | SQLite data directory |
| `TOON_DEV_MODE` | `false` | Skip event-signature verification on `POST /write` |

## Run (programmatic)

```ts
import { startRelay } from '@toon-protocol/relay';

const relay = await startRelay({ secretKey });
// ... POST /write on 3100, read NIP-01 on 7100 ...
await relay.stop();
```

## HTTP surface

| Method | Path | Description |
|--------|------|-------------|
| `POST` | `/write` | Store an event. Body `{ "event": <NostrEvent> }`. Trusts injected `X-TOON-Payer`/`-Amount`/`-Chain` headers (echoed, not validated); verifies only the event signature. |
| `GET`  | `/health` | Liveness, identity (`pubkey`), `capabilities`, and `version`. |

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

const memStore = new InMemoryEventStore();      // ephemeral
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

| Category | Exports |
|----------|---------|
| **Launcher** | `startRelay`, `RelayConfig`, `RelayInstance`, `RelaySubscription`, `ResolvedRelayConfig` |
| **Relay** | `NostrRelayServer`, `ConnectionHandler`, `RelayServerConfig`, `DEFAULT_RELAY_CONFIG` |
| **Storage** | `EventStore`, `InMemoryEventStore`, `SqliteEventStore`, `RelayError` |
| **Write/Health** | `createWriteHandler`, `createHealthResponse` |
| **Codec** | `encodeEventToToon`, `decodeEventFromToon`, `ToonEncodeError`, `ToonDecodeError` |
| **Subscriber** | `RelaySubscriber`, `RelaySubscriberConfig` |
| **Filter** | `matchFilter` |
| **Constants** | `VERSION` |

## License

MIT
