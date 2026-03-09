/**
 * @crosstown/town
 *
 * SDK-based relay with startTown() API and CLI for ILP-gated Nostr services.
 * Provides handler implementations and a one-call programmatic API for
 * starting a Crosstown relay node.
 */

// Town lifecycle API
export { startTown } from './town.js';
export type {
  TownConfig,
  TownInstance,
  TownSubscription,
  ResolvedTownConfig,
} from './town.js';

// Event storage handler
export { createEventStorageHandler } from './handlers/event-storage-handler.js';
export type { EventStorageHandlerConfig } from './handlers/event-storage-handler.js';
