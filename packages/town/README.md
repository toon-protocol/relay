# @crosstown/town

A complete Crosstown relay node — one command to run.

## What It Does

Town is the reference implementation of a Crosstown service. It combines the SDK's processing pipeline with an embedded ILP connector, WebSocket relay, SQLite storage, HTTP health endpoint, and automatic bootstrap into a single package.

Run it with one function call or one CLI command.

## Install

```bash
npm install @crosstown/town
```

## Usage

### CLI

```bash
npx @crosstown/town \
  --mnemonic "your twelve word mnemonic phrase here"
```

That's it. Town starts an embedded ILP connector, a WebSocket relay on port 7100, a BLS server on port 3100, discovers peers, opens payment channels, and begins accepting paid events.

### Programmatic

```typescript
import { startTown } from '@crosstown/town';

const town = await startTown({
  mnemonic: 'your twelve word mnemonic...',
  relayPort: 7100,
  blsPort: 3100,
  basePricePerByte: 10n,
});

console.log(`Relay: ws://localhost:${town.config.relayPort}`);
console.log(`Pubkey: ${town.pubkey}`);
console.log(`Peers: ${town.bootstrapResult.peerCount}`);

// Subscribe to another relay's events
const sub = town.subscribe('ws://other-relay:7100', { kinds: [1] });

// Shut down
await town.stop();
```

## Where It Sits

```
┌─────────────────────────┐
│  @crosstown/town        │  ← One-call relay (this package)
├─────────────────────────┤
│  @crosstown/sdk         │  ← Verification, pricing, handlers
├─────────────────────────┤
│  @crosstown/core        │  ← Bootstrap, discovery
│  @crosstown/relay       │  ← WebSocket server, event store
├─────────────────────────┤
│  Embedded Connector     │  ← ILP routing (included)
└─────────────────────────┘
```

Town is the SDK fully assembled. If you need custom handlers or different storage, use [`@crosstown/sdk`](../sdk) directly.

## Configuration

| Option | Default | Purpose |
|--------|---------|---------|
| `mnemonic` / `secretKey` | *required* | Node identity (provide one) |
| `relayPort` | `7100` | WebSocket relay port |
| `blsPort` | `3100` | BLS HTTP server port |
| `basePricePerByte` | `10n` | Price per byte of event data |
| `dataDir` | `./data` | SQLite database location |
| `devMode` | `false` | Skip verification and pricing |
| `knownPeers` | `[]` | Seed peers for bootstrap |
| `connectorUrl` | — | Use an external connector instead of the embedded one |

All options can also be set via environment variables (`CROSSTOWN_*`) or CLI flags (`--*`).

## What Happens on Start

1. Derives identity from mnemonic or secret key
2. Starts the embedded ILP connector
3. Creates SQLite event store in `dataDir`
4. Starts BLS HTTP server with `/health` and `/handle-packet`
5. Starts WebSocket relay (Nostr NIP-01)
6. Bootstraps into the network (discover → register → announce)
7. Returns `TownInstance` with lifecycle controls

## Full Documentation

See the [Town Guide](../../docs/town-guide.md) for the complete configuration reference, environment variables, CLI flags, TownInstance API, and startup sequence details.

## Requirements

- Node.js >= 20

## License

MIT
