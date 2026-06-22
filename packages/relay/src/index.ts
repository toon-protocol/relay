/**
 * @toon-protocol/relay
 *
 * Nostr relay app: free NIP-01 WebSocket reads plus an HTTP `POST /write`
 * surface for storing events. Payment is enforced upstream by an external
 * terminator, so this package contains no ILP/connector/settlement logic.
 */

export { VERSION } from './version.js';

// Types
// NOTE: the low-level WebSocket relay-server config is exported as
// `RelayServerConfig` (renamed from `RelayConfig`) so the launcher's
// `RelayConfig` can take the `RelayConfig` name.
export type { RelayServerConfig } from './types.js';
export { DEFAULT_RELAY_CONFIG } from './types.js';

// Storage
export type { EventStore } from './storage/index.js';
export {
  InMemoryEventStore,
  SqliteEventStore,
  RelayError,
} from './storage/index.js';

// Filters
export { matchFilter } from './filters/index.js';

// WebSocket
export type { Subscription } from './websocket/index.js';
export { ConnectionHandler, NostrRelayServer } from './websocket/index.js';

// TOON encoding/decoding
export {
  encodeEventToToon,
  decodeEventFromToon,
  ToonEncodeError,
  ToonDecodeError,
} from './toon/index.js';

// Subscriber
export type { RelaySubscriberConfig } from './subscriber/index.js';
export { RelaySubscriber } from './subscriber/index.js';

// ---------------------------------------------------------------------------
// Launcher
//
// One-call programmatic API (startRelay()) that wires the event store, the
// HTTP write/health server, and the NIP-01 WebSocket read server.
// ---------------------------------------------------------------------------

// Relay launcher lifecycle API
export { startRelay } from './launcher/relay.js';
export type {
  RelayConfig,
  RelayInstance,
  RelaySubscription,
  ResolvedRelayConfig,
} from './launcher/relay.js';

// Health response
export { createHealthResponse } from './launcher/health.js';
export type { HealthConfig, HealthResponse } from './launcher/health.js';

// Write handler (POST /write surface)
export { createWriteHandler } from './launcher/handlers/write-handler.js';
export type {
  WriteHandler,
  WriteHandlerConfig,
} from './launcher/handlers/write-handler.js';

// Re-exports from @toon-protocol/bls removed to avoid circular dependency
// Downstream consumers should import directly from @toon-protocol/bls instead
