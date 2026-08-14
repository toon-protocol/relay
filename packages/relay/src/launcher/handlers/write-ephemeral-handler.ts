/**
 * Free ephemeral write handler for @toon-protocol/relay (relay#129, ephemeral
 * epic toon-meta#393 E2).
 *
 * A SECOND write surface, `POST /write-ephemeral`, distinct from the paid
 * `POST /write` (write-handler.ts). It exists because the connector cannot
 * carry two prices on one `handler_url` (`ConflictingHandlerPrice`,
 * connector `route.rs:378-399`) -- a free lane needs its own endpoint,
 * terminated by its own zero-priced route in the deploy config (see
 * `deploy/connector.toml`'s `g.toon.relay.ephemeral` route).
 *
 * Differences from the paid handler, all deliberate:
 *
 * - Accepts ONLY ephemeral kinds (NIP-16, 20000 <= kind < 30000) -- anything
 *   else is a 400. Persistent kinds have no business on a free lane; letting
 *   them through would be a free ride around pay-to-write.
 * - NEVER stores. Ephemeral kinds are never persisted on the paid path
 *   either (NIP-16; write-handler.ts), so there is nothing this lane would
 *   ever write to an EventStore -- it does not take one as a dependency.
 * - Schnorr verification is ALWAYS FULL, with no skip and no config knob to
 *   add one. The paid path's ephemeral verify-skip (write-handler.ts) is
 *   safe ONLY because payment is the admission gate; this lane has no
 *   payment gate, so signature verification IS its only defense against
 *   forged-signature spam before the bounds below even apply. Reusing that
 *   skip here would let anyone broadcast garbage to every subscriber for
 *   free -- exactly the case write-handler.ts's own invariant comment warns
 *   against.
 * - Bounds, because free + broadcast = spam surface: a per-key sliding-
 *   window rate limit (rate-limiter.ts) and a request-body size cap, both
 *   config-gated with conservative defaults (see
 *   {@link EphemeralWriteHandlerConfig}).
 *
 * Flow:
 * 1. Rate-limit check, keyed by remote address (falling back to a shared
 *    bucket when connection info is unavailable -- see `defaultClientKey`)
 *    -- BEFORE any body is read, so a rate-limited caller costs as little
 *    work as possible -> 429 over budget.
 * 2. Body-size check against `maxBodyBytes` -- BEFORE JSON parsing, so an
 *    oversized payload is never deserialized -> 413 too large.
 * 3. Parse JSON body `{ event }` -> 400 on malformed/missing event.
 * 4. Reject non-ephemeral kinds -> 400.
 * 5. Full schnorr verification, never skipped -> 422 on invalid signature.
 * 6. Fire the optional onBroadcast callback (the live-broadcast hook).
 *    Nothing is ever stored.
 * 7. Respond 200 with the event id.
 *
 * @module
 */

import type { Context } from 'hono';
import { getConnInfo } from '@hono/node-server/conninfo';
import type { NostrEvent } from 'nostr-tools/pure';
import { verifyEventSignature } from '../../crypto/index.js';
import { createRateLimiter } from '../rate-limiter.js';
import type { RateLimiter } from '../rate-limiter.js';

/**
 * Whether `kind` is ephemeral per NIP-16 (20000 <= kind < 30000) -- the ONLY
 * kinds this lane accepts.
 */
function isEphemeralKind(kind: number): boolean {
  return kind >= 20000 && kind < 30000;
}

/**
 * Conservative default rate-limit bound: 200 requests per 10-second window
 * per key. Sized for presence/typing traffic (the epic's motivating
 * workload, toon-meta#393), not huddle-frame rates -- that traffic stays on
 * the paid path. Deliberately generous rather than tight: this is a spam
 * ceiling, not a fairness scheduler, and a false-positive reject on
 * legitimate ephemeral traffic (a dropped typing indicator) is silent and
 * has no client-side retry signal.
 */
export const DEFAULT_EPHEMERAL_RATE_LIMIT: {
  maxRequests: number;
  windowMs: number;
} = {
  maxRequests: 200,
  windowMs: 10_000,
};

/**
 * Conservative default body-size cap in bytes. Ephemeral events on this lane
 * (presence heartbeats, typing indicators) are small JSON; 8 KiB comfortably
 * covers a signed Nostr event with generous tag/content headroom while
 * bounding worst-case memory per request on an unpaid surface.
 */
export const DEFAULT_EPHEMERAL_MAX_BODY_BYTES = 8 * 1024;

/** Configuration for the ephemeral write handler. */
export interface EphemeralWriteHandlerConfig {
  /**
   * Signature verifier. Defaults to the inline `verifyEventSignature`; the
   * launcher injects the worker-pool verifier (crypto/verify-pool.ts),
   * shared with the paid handler, so verify bursts run off the event loop
   * (relay#85). May resolve asynchronously -- the handler awaits it.
   *
   * There is NO devMode/skip option here, unlike the paid handler --
   * verification on this lane is always full (see the module doc).
   */
  verifyEvent?: (event: NostrEvent) => Promise<boolean> | boolean;
  /** Optional callback fired after an event passes all checks (broadcast hook). */
  onBroadcast?: (event: NostrEvent) => void;
  /** Log one line per accepted write (default: false), matching write-handler.ts. */
  logWrites?: boolean;
  /** Rate-limit bound (default {@link DEFAULT_EPHEMERAL_RATE_LIMIT}). */
  rateLimit?: { maxRequests: number; windowMs: number };
  /** Request body size cap in bytes (default {@link DEFAULT_EPHEMERAL_MAX_BODY_BYTES}). */
  maxBodyBytes?: number;
  /**
   * Test-only: inject a rate limiter directly (e.g. with a fake clock)
   * instead of letting the handler build one from `rateLimit`.
   */
  rateLimiter?: RateLimiter;
  /**
   * Test-only: override how a request is keyed for rate limiting. Defaults
   * to `defaultClientKey` (remote address via `getConnInfo`, falling back to
   * a shared bucket).
   */
  getClientKey?: (c: Context) => string;
}

/** Ephemeral write handler instance. */
export interface EphemeralWriteHandler {
  /** Handle a plain-HTTP ephemeral write request. */
  handleWrite(c: Context): Promise<Response>;
}

/**
 * Default rate-limit key: the caller's remote address via `@hono/node-server`'s
 * connection-info helper. Falls back to a single shared bucket ('unknown')
 * when connection info isn't available -- e.g. a bare `app.fetch()` call in
 * unit tests, or any transport that doesn't expose a socket. That fallback
 * degrades to a global cap rather than an unbounded one, which is the safe
 * direction for a rate limiter to fail.
 *
 * In production this handler sits behind the connector (deploy/docker-compose.yml
 * never host-publishes :3100), so the observed remote address is the
 * connector's own -- the connector does not forward per-client identity to
 * the relay (`toon-protocol/connector` ADR 0006/0036) -- making this a
 * de facto lane-wide cap in the canonical deploy, not a true per-end-user
 * one. That is a known, accepted shape: the bound exists to cap the blast
 * radius of the free lane as a whole, not to fairness-schedule individual
 * end users.
 *
 * @internal Exported for unit testing.
 */
export function defaultClientKey(c: Context): string {
  try {
    return getConnInfo(c).remote.address ?? 'unknown';
  } catch {
    return 'unknown';
  }
}

/**
 * Create the ephemeral write handler.
 *
 * @param config - Handler configuration.
 * @returns An EphemeralWriteHandler with a handleWrite method.
 */
export function createEphemeralWriteHandler(
  config: EphemeralWriteHandlerConfig = {}
): EphemeralWriteHandler {
  const logWrites = config.logWrites ?? false;
  const verifyEvent = config.verifyEvent ?? verifyEventSignature;
  const maxBodyBytes = config.maxBodyBytes ?? DEFAULT_EPHEMERAL_MAX_BODY_BYTES;
  const rateLimiter =
    config.rateLimiter ??
    createRateLimiter(config.rateLimit ?? DEFAULT_EPHEMERAL_RATE_LIMIT);
  const getClientKey = config.getClientKey ?? defaultClientKey;

  return {
    async handleWrite(c: Context): Promise<Response> {
      // --- Rate limit (before any body is read) ---
      if (!rateLimiter.allow(getClientKey(c))) {
        return c.json({ error: 'Rate limit exceeded' }, 429);
      }

      // --- Size cap (before JSON parsing) ---
      const contentLengthHeader = c.req.header('content-length');
      if (
        contentLengthHeader !== undefined &&
        Number(contentLengthHeader) > maxBodyBytes
      ) {
        return c.json({ error: 'Request body too large' }, 413);
      }

      let rawBody: string;
      try {
        rawBody = await c.req.text();
      } catch {
        return c.json({ error: 'Invalid request body' }, 400);
      }
      if (Buffer.byteLength(rawBody, 'utf8') > maxBodyBytes) {
        return c.json({ error: 'Request body too large' }, 413);
      }

      // --- Parse request body ---
      let body: { event?: NostrEvent };
      try {
        body = JSON.parse(rawBody) as { event?: NostrEvent };
      } catch {
        return c.json({ error: 'Invalid request body' }, 400);
      }

      if (!body.event) {
        return c.json({ error: 'Missing required field: event' }, 400);
      }

      const event = body.event;

      // --- Ephemeral-only gate: reject anything else before spending a
      // verify on it. A non-ephemeral kind on this lane would be a free ride
      // around pay-to-write. ---
      if (!isEphemeralKind(event.kind)) {
        return c.json(
          {
            error:
              'Only ephemeral kinds (20000-29999) are accepted on this lane',
          },
          400
        );
      }

      if (logWrites) {
        console.log(`[write] event=${event.id} handler=write-ephemeral`);
      }

      // --- Full schnorr verification -- NEVER skipped on this lane (relay#129) ---
      if (!(await verifyEvent(event))) {
        return c.json({ error: 'Invalid event signature' }, 422);
      }

      // --- Broadcast only; nothing is ever stored (NIP-16) ---
      config.onBroadcast?.(event);

      return c.json(
        {
          eventId: event.id,
          broadcastAt: Math.floor(Date.now() / 1000),
        },
        200
      );
    },
  };
}
