/**
 * Write handler for @toon-protocol/relay.
 *
 * Exposes a plain-HTTP write surface that accepts an event-as-JSON, trusts
 * (but does NOT validate) injected payment headers, verifies ONLY the event
 * signature for integrity, and stores the event.
 *
 * This handler is intentionally decoupled from any payment layer: it contains
 * no claim/settlement/ILP logic and imports none of it. Payment validation is
 * the upstream terminator's concern; by the time a request reaches this surface
 * the trusted `X-TOON-*` headers are assumed already proven. The handler
 * captures them purely for the response echo and a log line.
 *
 * Flow:
 * 1. Parse JSON body `{ event }` -> 400 on malformed/missing event
 * 2. Capture trusted X-TOON-Payer / X-TOON-Amount / X-TOON-Chain headers
 * 3. Verify the event signature (skipped in devMode) -> 422 on invalid sig
 * 4. Store the event in the EventStore
 * 5. Fire the optional onStored callback
 * 6. Respond 200 with the event id, storedAt timestamp, and echoed headers
 *
 * @module
 */

import type { Context } from 'hono';
import { verifyEvent } from 'nostr-tools/pure';
import type { NostrEvent } from 'nostr-tools/pure';
import type { EventStore } from '../../storage/index.js';

/**
 * Configuration for the write handler.
 */
export interface WriteHandlerConfig {
  /** Event store backend used to persist accepted events. */
  eventStore: EventStore;
  /** Whether dev mode is enabled (skips Schnorr signature verification). */
  devMode: boolean;
  /** Optional callback fired after an event is successfully stored. */
  onStored?: (event: NostrEvent) => void;
}

/**
 * Write handler instance.
 */
export interface WriteHandler {
  /** Handle a plain-HTTP write request. */
  handleWrite(c: Context): Promise<Response>;
}

/**
 * Create a write handler.
 *
 * @param config - Handler configuration.
 * @returns A WriteHandler with a handleWrite method.
 */
export function createWriteHandler(
  config: WriteHandlerConfig
): WriteHandler {
  return {
    async handleWrite(c: Context): Promise<Response> {
      // --- Parse request body ---
      let body: { event?: NostrEvent };
      try {
        body = (await c.req.json()) as { event?: NostrEvent };
      } catch {
        return c.json({ error: 'Invalid request body' }, 400);
      }

      if (!body.event) {
        return c.json({ error: 'Missing required field: event' }, 400);
      }

      const event = body.event;

      // --- Capture trusted payment headers (NOT validated here) ---
      const payer = c.req.header('X-TOON-Payer');
      const amount = c.req.header('X-TOON-Amount');
      const chain = c.req.header('X-TOON-Chain');

      console.log(
        `[write] event=${event.id} payer=${payer ?? '-'} amount=${amount ?? '-'} chain=${chain ?? '-'}`
      );

      // --- Verify event signature (integrity only; skipped in devMode) ---
      if (!config.devMode && !verifyEvent(event)) {
        return c.json({ error: 'Invalid event signature' }, 422);
      }

      // --- Store the event ---
      config.eventStore.store(event);

      // --- Fire the optional stored callback ---
      config.onStored?.(event);

      // --- Build response (echo trusted headers) ---
      return c.json(
        {
          eventId: event.id,
          storedAt: Math.floor(Date.now() / 1000),
          payer,
          amount,
          chain,
        },
        200
      );
    },
  };
}
