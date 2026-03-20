# @toon-protocol/relay

ILP-gated Nostr relay with payment verification, event storage, and dynamic pricing.

> This is an internal package. Most users should start with [`@toon-protocol/town`](../town) which wraps this into a single command, or [`@toon-protocol/sdk`](../sdk) for custom services.

## Install

```bash
npm install @toon-protocol/relay
```

## WebSocket Relay Server

NIP-01 compliant WebSocket server that stores and serves Nostr events in TOON format.

```ts
import { NostrRelayServer, SqliteEventStore } from '@toon-protocol/relay';

const eventStore = new SqliteEventStore('./events.db');
const relay = new NostrRelayServer({ port: 7100 }, eventStore);

await relay.start();
console.log(`Relay listening on port ${relay.getPort()}`);
console.log(`Connected clients: ${relay.getClientCount()}`);

// Push an event to all matching subscriptions
relay.broadcastEvent(event);

await relay.stop();
```

### Configuration

```ts
import { DEFAULT_RELAY_CONFIG } from '@toon-protocol/relay';

const config = {
  port: 7100,                          // Default: 7000
  maxConnections: 100,                 // Default: 100
  maxSubscriptionsPerConnection: 20,   // Default: 20
  maxFiltersPerSubscription: 10,       // Default: 10
  databasePath: './events.db',         // Default: ':memory:'
};
```

## Event Storage

Two built-in storage backends.

```ts
import { InMemoryEventStore, SqliteEventStore } from '@toon-protocol/relay';

// In-memory (for testing or ephemeral nodes)
const memStore = new InMemoryEventStore();

// SQLite (for persistent storage)
const sqlStore = new SqliteEventStore('./events.db');

// Both implement the EventStore interface
memStore.store(event);
const found = memStore.get(event.id);
const results = memStore.query([{ kinds: [1], limit: 10 }]);
```

## Business Logic Server (BLS)

HTTP server that verifies ILP payment packets, validates events, and gates writes behind payment.

```ts
import { BusinessLogicServer, SqliteEventStore } from '@toon-protocol/relay';

const eventStore = new SqliteEventStore('./events.db');
const bls = new BusinessLogicServer({
  basePricePerByte: 10n,
  ownerPubkey: 'abc123...',   // Owner events bypass payment
}, eventStore);

// Start HTTP server
bls.listen(3100);

// Or handle packets directly
const response = bls.handlePacket({
  amount: '5000',
  destination: 'g.toon.node-a',
  data: 'base64-encoded-toon-event',
});

if (response.accept) {
  console.log('Event stored:', response.metadata?.eventId);
}
```

### Endpoints

| Method | Path | Description |
|--------|------|-------------|
| `GET` | `/health` | Health check with pricing, capabilities, chain info |
| `POST` | `/handle-packet` | Process ILP payment packets |

## Pricing Service

Dynamic per-byte pricing with kind-specific overrides.

```ts
import { PricingService } from '@toon-protocol/relay';

const pricing = new PricingService({
  basePricePerByte: 10n,
  kindOverrides: new Map([
    [1, 5n],      // Short notes: cheaper
    [30023, 20n], // Long-form articles: premium
  ]),
});

const price = pricing.computePrice(1, eventSizeInBytes);
```

## TOON Codec

Re-exported from [`@toon-protocol/core`](../core) for convenience.

```ts
import { encodeEventToToon, decodeEventFromToon } from '@toon-protocol/relay';

const toonBytes = encodeEventToToon(nostrEvent);
const event = decodeEventFromToon(toonBytes);
```

## Full API

| Category | Exports |
|----------|---------|
| **Relay** | `NostrRelayServer`, `ConnectionHandler`, `RelayConfig`, `DEFAULT_RELAY_CONFIG` |
| **Storage** | `EventStore`, `InMemoryEventStore`, `SqliteEventStore` |
| **BLS** | `BusinessLogicServer`, `BlsConfig`, `HandlePacketRequest/Response` |
| **Pricing** | `PricingService`, `PricingConfig`, `loadPricingConfigFromEnv`, `loadPricingConfigFromFile` |
| **Codec** | `encodeEventToToon`, `decodeEventFromToon`, `ToonEncodeError`, `ToonDecodeError` |
| **Filter** | `matchFilter` |
| **Errors** | `RelayError`, `BlsError`, `PricingError` |
| **Constants** | `ILP_ERROR_CODES`, `PUBKEY_REGEX`, `VERSION` |

## License

MIT
