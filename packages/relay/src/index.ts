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
export type { EventStore, EventStoreOptions } from './storage/index.js';
export {
  InMemoryEventStore,
  SqliteEventStore,
  RelayError,
} from './storage/index.js';

// Filters
export { matchFilter } from './filters/index.js';

// Retention policy: NIP-40 expiration, NIP-09 deletion, operator blocklist
export {
  EXPIRATION_TAG,
  getExpiration,
  isExpired,
  DELETION_KIND,
  isDeletionKind,
  isDeletableBy,
  parseAddressCoordinate,
  parseDeletionTargets,
  parseBlockedEventIds,
} from './nips/index.js';
export type { AddressCoordinate, DeletionTargets } from './nips/index.js';

// Crypto (fast BIP-340 event verification, relay#85)
export {
  verifyEventSignature,
  verifyEventId,
  verifyImplementation,
} from './crypto/index.js';

// Verify worker pool (verify off the event loop, relay#85)
export {
  createVerifyPool,
  defaultVerifyWorkers,
} from './crypto/verify-pool.js';
export type { VerifyPool, VerifyPoolOptions } from './crypto/verify-pool.js';

// WebSocket
export type { Subscription } from './websocket/index.js';
export {
  ConnectionHandler,
  NostrRelayServer,
  serializeEventFrame,
  readOpenFilesSoftLimit,
} from './websocket/index.js';

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
export {
  startRelay,
  isInternalBindHost,
  warnIfWritePortExposed,
} from './launcher/relay.js';
export type {
  RelayConfig,
  RelayInstance,
  RelaySubscription,
  ResolvedRelayConfig,
} from './launcher/relay.js';

// Health response
export { createHealthResponse } from './launcher/health.js';
export type { HealthConfig, HealthResponse } from './launcher/health.js';

// Metrics (GET /metrics telemetry surface, relay#85)
export { createMetricsRegistry } from './launcher/metrics.js';
export type {
  MetricsRegistry,
  MetricsSnapshot,
  DurationStats,
} from './launcher/metrics.js';

// Write handler (POST /write surface)
export { createWriteHandler } from './launcher/handlers/write-handler.js';
export type {
  WriteHandler,
  WriteHandlerConfig,
} from './launcher/handlers/write-handler.js';

// The connector's payment statement, as read off a delivery (ADR 0040)
export { readPaymentAttribution } from './launcher/handlers/payment-attribution.js';
export type { PaymentAttribution } from './launcher/handlers/payment-attribution.js';

// Ephemeral write handler (POST /write-ephemeral surface, relay#129)
export {
  createEphemeralWriteHandler,
  DEFAULT_EPHEMERAL_RATE_LIMIT,
  DEFAULT_EPHEMERAL_MAX_BODY_BYTES,
} from './launcher/handlers/write-ephemeral-handler.js';
export type {
  EphemeralWriteHandler,
  EphemeralWriteHandlerConfig,
} from './launcher/handlers/write-ephemeral-handler.js';

// Rate limiter (relay#129) backing the ephemeral write lane's bounds
export { createRateLimiter } from './launcher/rate-limiter.js';
export type {
  RateLimiter,
  RateLimiterOptions,
} from './launcher/rate-limiter.js';

// Re-exports from @toon-protocol/bls removed to avoid circular dependency
// Downstream consumers should import directly from @toon-protocol/bls instead
