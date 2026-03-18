/**
 * @toon-protocol/town
 *
 * SDK-based relay with startTown() API and CLI for ILP-gated Nostr services.
 * Provides handler implementations and a one-call programmatic API for
 * starting a TOON relay node.
 */

// Town lifecycle API
export { startTown } from './town.js';
export type {
  TownConfig,
  TownInstance,
  TownSubscription,
  ResolvedTownConfig,
} from './town.js';

// Health response (Story 3.6)
export { createHealthResponse } from './health.js';
export type { HealthConfig, HealthResponse, TeeHealthInfo } from './health.js';

// Event storage handler
export { createEventStorageHandler } from './handlers/event-storage-handler.js';
export type { EventStorageHandlerConfig } from './handlers/event-storage-handler.js';

// x402 publish handler
export { createX402Handler } from './handlers/x402-publish-handler.js';
export type {
  X402HandlerConfig,
  X402Handler,
} from './handlers/x402-publish-handler.js';

// x402 pricing
export { calculateX402Price } from './handlers/x402-pricing.js';
export type { X402PricingConfig } from './handlers/x402-pricing.js';

// x402 pre-flight
export { runPreflight } from './handlers/x402-preflight.js';
export type {
  PreflightResult,
  PreflightConfig,
} from './handlers/x402-preflight.js';

// x402 settlement
export { settleEip3009 } from './handlers/x402-settlement.js';
export type {
  X402SettlementResult,
  X402SettlementConfig,
} from './handlers/x402-settlement.js';
// Deprecated aliases -- use X402SettlementResult / X402SettlementConfig instead
export type {
  SettlementResult,
  SettlementConfig,
} from './handlers/x402-settlement.js';

// x402 types
export type {
  Eip3009Authorization,
  EventStoreLike,
  X402PublishRequest,
  X402PublishResponse,
  X402PricingResponse,
} from './handlers/x402-types.js';
export {
  EIP_3009_TYPES,
  USDC_EIP712_DOMAIN,
  USDC_ABI,
} from './handlers/x402-types.js';
