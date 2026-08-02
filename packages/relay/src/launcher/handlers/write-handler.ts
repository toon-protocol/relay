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
 * 3. Verify the event signature (skipped in devMode) -> 422 on invalid sig.
 *    Verification uses the fast WASM libsecp256k1 path (crypto/verify-event)
 *    rather than noble pure-JS: post-#84 the synchronous ~1.3ms noble verify
 *    on the single event loop WAS the write-path ceiling (~240-260 events/s
 *    aggregate, relay#85 / connector#685 Phase G).
 *    PAID-EPHEMERAL EXCEPTION (relay#85, decision 2026-08-02): for ephemeral
 *    kinds (20000 <= kind < 30000) the schnorr verification is SKIPPED by
 *    default -- only the SHA-256 id check runs -> 422 on id mismatch. See
 *    the loud comment at the verify step for why this is safe, and
 *    `verifyEphemeral` to turn full verification back on.
 * 4. Store the event in the EventStore -- unless its kind is ephemeral
 *    (NIP-16: 20000 <= kind < 30000), which is delivered live and never
 *    persisted. Skipping the store here is not only NIP-16 semantics: the
 *    synchronous per-event disk write was the serialization point that
 *    capped the whole paid-write pipeline at ~150 events/s globally
 *    (connector#685), and ephemeral traffic -- audio frames -- is exactly
 *    the traffic that hits that path hardest.
 * 5. Fire the optional onStored callback (ephemeral events included: it is
 *    the live-broadcast hook, and ephemeral events exist only as that
 *    broadcast)
 * 6. Respond 200 with the event id, storedAt timestamp, and echoed headers
 *
 * @module
 */

import type { Context } from 'hono';
import type { NostrEvent } from 'nostr-tools/pure';
import { verifyEventSignature, verifyEventId } from '../../crypto/index.js';
import type { EventStore } from '../../storage/index.js';

/**
 * Whether `kind` is ephemeral per NIP-16 (20000 <= kind < 30000): delivered
 * to live subscribers but never persisted or served from REQ history.
 */
function isEphemeralKind(kind: number): boolean {
  return kind >= 20000 && kind < 30000;
}

/**
 * Configuration for the write handler.
 */
export interface WriteHandlerConfig {
  /** Event store backend used to persist accepted events. */
  eventStore: EventStore;
  /** Whether dev mode is enabled (skips Schnorr signature verification). */
  devMode: boolean;
  /**
   * Run FULL schnorr verification on ephemeral kinds too (default: false --
   * i.e. the paid-ephemeral verify skip is ON by default).
   *
   * ! SECURITY INVARIANT -- READ BEFORE TOUCHING !
   * The default skip is safe ONLY because this write surface is payment-gated:
   * every request reaching `POST /write` has already passed the upstream
   * connector's claim gate (payment IS the admission/spam gate), and the
   * protocol rule is that clients trust the signature chain and verify every
   * event themselves -- never the relay. Relay-side schnorr on ephemeral
   * frames is therefore pure spam defense that payment already provides;
   * forging a speaker costs real money to emit frames every client discards.
   * The SHA-256 id check is ALWAYS kept (see handleWrite).
   *
   * If you ever add a FREE (non-payment-gated) ephemeral write lane, it MUST
   * NOT reuse this skip -- free spam with valid-looking ids would be
   * broadcast to every subscriber. Community operators who front this port
   * with anything other than a payment-gating connector should set
   * `verifyEphemeral: true` (TOON_VERIFY_EPHEMERAL=true).
   */
  verifyEphemeral?: boolean;
  /**
   * Signature verifier for non-skipped (persistent-kind) events. Defaults to
   * the inline `verifyEventSignature`; the launcher injects the worker-pool
   * verifier (`crypto/verify-pool.ts`) so verify bursts run off the event
   * loop (relay#85). May resolve asynchronously -- the handler awaits it.
   *
   * ORDERING NOTE: an async verifier means CONCURRENT requests can complete
   * out of arrival order. Per-session write ordering is enforced UPSTREAM:
   * the connector serializes each BTP session's POSTs (it does not send the
   * next request until the previous response arrives), so sequential
   * same-session writes can never reorder here. Pinned by the ordering test
   * in write-handler.test.ts -- do not weaken that contract upstream without
   * revisiting this.
   */
  verifyEvent?: (event: NostrEvent) => Promise<boolean> | boolean;
  /** Optional callback fired after an event is successfully stored. */
  onStored?: (event: NostrEvent) => void;
  /**
   * Log one line per accepted write (default: false). Off by default because
   * per-event console I/O on the single event loop is measurable tail jitter
   * at huddle frame rates (relay#85, connector#685 Phase G): every write's
   * log line goes through docker's json-file driver, i.e. residual per-event
   * disk I/O that #84 did not remove.
   */
  logWrites?: boolean;
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
export function createWriteHandler(config: WriteHandlerConfig): WriteHandler {
  const logWrites = config.logWrites ?? false;
  const verifyEphemeral = config.verifyEphemeral ?? false;
  const verifyEvent = config.verifyEvent ?? verifyEventSignature;
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

      if (logWrites) {
        console.log(
          `[write] event=${event.id} payer=${payer ?? '-'} amount=${amount ?? '-'} chain=${chain ?? '-'}`
        );
      }

      // --- Verify event signature (integrity only; skipped in devMode) ---
      //
      // !!! PAYMENT-GATED VERIFY BYPASS (relay#85, decided 2026-08-02) !!!
      // Ephemeral kinds (NIP-16, 20000 <= kind < 30000) skip schnorr entirely
      // by default: payment is already the admission gate (the upstream
      // connector's claim gate), and clients verify every signature
      // themselves -- the relay's verdict is never trusted. We KEEP the
      // SHA-256 id check so the broadcast bytes always match the id clients
      // index/verify by. This bypass is safe ONLY because POST /write is
      // payment-gated; a future FREE ephemeral lane MUST NOT reuse it.
      // Operators can restore full verification with verifyEphemeral
      // (TOON_VERIFY_EPHEMERAL=true).
      if (!config.devMode) {
        if (isEphemeralKind(event.kind) && !verifyEphemeral) {
          if (!verifyEventId(event)) {
            return c.json({ error: 'Invalid event id' }, 422);
          }
        } else if (!(await verifyEvent(event))) {
          return c.json({ error: 'Invalid event signature' }, 422);
        }
      }

      // --- Store the event (ephemeral kinds are broadcast-only, NIP-16) ---
      if (!isEphemeralKind(event.kind)) {
        config.eventStore.store(event);
      }

      // --- Fire the optional stored callback (the live-broadcast hook) ---
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
