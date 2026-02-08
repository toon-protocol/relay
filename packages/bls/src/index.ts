/**
 * @agent-society/bls
 *
 * Standalone Business Logic Server for ILP-gated Nostr event storage.
 */

// Server factory
export { createBlsServer } from './server.js';
export type { CreateBlsServerConfig, BlsServerInstance } from './server.js';

// Configuration
export type { BlsEnvConfig } from './config.js';
export { loadBlsConfigFromEnv } from './config.js';

// Errors
export { BlsBaseError, ConfigError } from './errors.js';

// Business Logic Server
export type {
  BlsConfig,
  HandlePaymentRequest,
  HandlePaymentAcceptResponse,
  HandlePaymentRejectResponse,
  HandlePaymentResponse,
} from './bls/index.js';
export {
  BlsError,
  ILP_ERROR_CODES,
  isValidPubkey,
  BusinessLogicServer,
  generateFulfillment,
} from './bls/index.js';

// Storage
export type { EventStore } from './storage/index.js';
export { InMemoryEventStore } from './storage/index.js';
export { SqliteEventStore } from './storage/index.js';

// TOON encoding/decoding
export {
  encodeEventToToon,
  decodeEventFromToon,
  ToonEncodeError,
  ToonError,
} from './toon/index.js';

// Pricing
export type { PricingConfig } from './pricing/index.js';
export {
  PricingError,
  PricingService,
  loadPricingConfigFromEnv,
  loadPricingConfigFromFile,
} from './pricing/index.js';
