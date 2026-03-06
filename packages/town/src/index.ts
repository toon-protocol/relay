/**
 * @crosstown/town
 *
 * SDK-based relay handlers for ILP-gated Nostr services.
 * Provides handler implementations that run on top of @crosstown/sdk.
 */

// Event storage handler
export { createEventStorageHandler } from './handlers/event-storage-handler.js';
export type { EventStorageHandlerConfig } from './handlers/event-storage-handler.js';

// SPSP handshake handler
export { createSpspHandshakeHandler } from './handlers/spsp-handshake-handler.js';
export type { SpspHandshakeHandlerConfig } from './handlers/spsp-handshake-handler.js';
