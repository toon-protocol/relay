/**
 * @toon-protocol/relay
 *
 * ILP-gated Nostr relay with Business Logic Server.
 */

export const VERSION = '0.1.0';

// Types
// NOTE: the low-level WebSocket relay-server config is exported as
// `RelayServerConfig` (renamed from `RelayConfig`) so the launcher's
// `RelayConfig` (formerly `TownConfig`) can take the `RelayConfig` name.
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

// Business Logic Server
export type {
  BlsConfig,
  HandlePacketRequest,
  HandlePacketAcceptResponse,
  HandlePacketRejectResponse,
  HandlePacketResponse,
} from './bls/index.js';
export {
  BlsError,
  ILP_ERROR_CODES,
  BusinessLogicServer,
  isValidPubkey,
} from './bls/index.js';

// Pricing
export type { PricingConfig } from './pricing/index.js';
export {
  PricingError,
  PricingService,
  loadPricingConfigFromEnv,
  loadPricingConfigFromFile,
} from './pricing/index.js';

// Subscriber
export type { RelaySubscriberConfig } from './subscriber/index.js';
export { RelaySubscriber } from './subscriber/index.js';

// ---------------------------------------------------------------------------
// Launcher (formerly @toon-protocol/town)
//
// SDK-based relay launcher with a one-call programmatic API (startRelay()) and
// handler implementations for ILP-gated Nostr services. Merged in from the
// @toon-protocol/town package. Old `startTown`/`Town*` names are re-exported
// as deprecated aliases.
// ---------------------------------------------------------------------------

// Relay launcher lifecycle API
export { startRelay } from './launcher/town.js';
export type {
  RelayConfig,
  RelayInstance,
  RelaySubscription,
  ResolvedRelayConfig,
} from './launcher/town.js';

// Deprecated launcher aliases (town → relay merge)
export { startTown } from './launcher/town.js';
export type {
  TownConfig,
  TownInstance,
  TownSubscription,
  ResolvedTownConfig,
} from './launcher/town.js';

// Health response
export { createHealthResponse } from './launcher/health.js';
export type {
  HealthConfig,
  HealthResponse,
  TeeHealthInfo,
} from './launcher/health.js';

// Event storage handler
export { createEventStorageHandler } from './launcher/handlers/event-storage-handler.js';
export type { EventStorageHandlerConfig } from './launcher/handlers/event-storage-handler.js';

// x402 publish handler
export { createX402Handler } from './launcher/handlers/x402-publish-handler.js';
export type {
  X402HandlerConfig,
  X402Handler,
} from './launcher/handlers/x402-publish-handler.js';

// x402 pricing
export { calculateX402Price } from './launcher/handlers/x402-pricing.js';
export type { X402PricingConfig } from './launcher/handlers/x402-pricing.js';

// x402 pre-flight
export { runPreflight } from './launcher/handlers/x402-preflight.js';
export type {
  PreflightResult,
  PreflightConfig,
} from './launcher/handlers/x402-preflight.js';

// x402 settlement
export { settleEip3009 } from './launcher/handlers/x402-settlement.js';
export type {
  X402SettlementResult,
  X402SettlementConfig,
} from './launcher/handlers/x402-settlement.js';
// Deprecated aliases -- use X402SettlementResult / X402SettlementConfig instead
export type {
  SettlementResult,
  SettlementConfig,
} from './launcher/handlers/x402-settlement.js';

// x402 types
export type {
  Eip3009Authorization,
  EventStoreLike,
  X402PublishRequest,
  X402PublishResponse,
  X402PricingResponse,
} from './launcher/handlers/x402-types.js';
export {
  EIP_3009_TYPES,
  USDC_EIP712_DOMAIN,
  USDC_ABI,
} from './launcher/handlers/x402-types.js';

// Re-exports from @toon-protocol/bls removed to avoid circular dependency
// Downstream consumers should import directly from @toon-protocol/bls instead
