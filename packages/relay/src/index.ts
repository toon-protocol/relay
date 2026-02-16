/**
 * @agent-society/relay
 *
 * ILP-gated Nostr relay with Business Logic Server.
 */

export const VERSION = '0.1.0';

// Types
export type { RelayConfig } from './types.js';
export { DEFAULT_RELAY_CONFIG } from './types.js';

// Storage
export type { EventStore } from './storage/index.js';
export { InMemoryEventStore, SqliteEventStore, RelayError } from './storage/index.js';

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
  ToonError,
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
  generateFulfillment,
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

// Re-export from @agent-society/bls for downstream consumers
export {
  BlsBaseError,
  createBlsServer,
} from '@agent-society/bls';
export type {
  CreateBlsServerConfig,
  BlsServerInstance,
} from '@agent-society/bls';
