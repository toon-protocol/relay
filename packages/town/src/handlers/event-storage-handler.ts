/**
 * Event storage handler for @toon-protocol/town.
 *
 * Stores incoming Nostr events in the EventStore after decoding from TOON.
 * This is the "default" handler for the relay -- it processes all event kinds
 * except those handled by kind-specific handlers.
 *
 * The handler is intentionally simple (~15 lines of logic). The SDK pipeline
 * handles signature verification, pricing validation, and self-write bypass
 * before the handler is invoked. The handler only needs to:
 *   1. ctx.decode() -- lazy-decode the TOON payload into a NostrEvent
 *   2. eventStore.store(event) -- persist the event
 *   3. ctx.accept({ eventId, storedAt }) -- accept the ILP packet
 */

import type { EventStore } from '@toon-protocol/relay';
import type {
  Handler,
  HandlerContext,
  HandlerResponse,
} from '@toon-protocol/sdk';

/**
 * Configuration for the event storage handler.
 *
 * Minimal by design -- the handler's only job is decode + store + accept.
 * Pricing, verification, and self-write bypass are SDK pipeline concerns.
 */
export interface EventStorageHandlerConfig {
  /** Event store backend (e.g., SqliteEventStore from @toon-protocol/relay). */
  eventStore: EventStore;
}

/**
 * Creates an event storage handler that decodes TOON payloads and stores
 * Nostr events in the configured EventStore.
 *
 * Errors from `ctx.decode()` or `eventStore.store()` are not caught here --
 * they propagate to the SDK's dispatch error boundary, which converts
 * unhandled exceptions to `{ accept: false, code: 'T00', message: 'Internal error' }`.
 *
 * @param config - Handler configuration with the event store backend.
 * @returns A handler function compatible with `node.onDefault(handler)`.
 */
export function createEventStorageHandler(
  config: EventStorageHandlerConfig
): Handler {
  const { eventStore } = config;

  return async (ctx: HandlerContext): Promise<HandlerResponse> => {
    // Decode the TOON payload into a structured NostrEvent
    const event = ctx.decode();

    // Store the event (EventStore handles replaceable events, duplicates, etc.)
    eventStore.store(event);

    // Accept the packet with event metadata
    return ctx.accept({ eventId: event.id, storedAt: Date.now() });
  };
}
