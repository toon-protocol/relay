/**
 * Unit tests for the write handler.
 *
 * The handler accepts an event-as-JSON, verifies only the event signature
 * for integrity, and stores the event. No payment headers are read or
 * echoed -- the terminating connector asserts nothing about payment to this
 * relay (`toon-protocol/connector` ADR 0036). These tests cover:
 *
 * - valid signed event -> 200, event stored, onStored called exactly once
 * - malformed / missing body -> 400
 * - invalid signature (non-dev) -> 422; same bad event with devMode -> 200
 * - retired X-TOON-* headers from a stale caller -> read-and-ignored, never
 *   echoed back
 * - per-write console logging is OFF by default and opt-in via logWrites
 *   (per-event console I/O is write-path tail jitter, relay#85)
 */

import { describe, it, expect, vi, afterEach } from 'vitest';
import { Hono } from 'hono';
import { generateSecretKey, finalizeEvent } from 'nostr-tools/pure';
import type { NostrEvent } from 'nostr-tools/pure';
import { InMemoryEventStore } from '../../storage/index.js';
import { createWriteHandler } from './write-handler.js';
import type { WriteHandlerConfig } from './write-handler.js';

/**
 * Create a properly signed Nostr event for testing.
 */
function createValidSignedEvent(
  overrides: Partial<Omit<NostrEvent, 'id' | 'sig' | 'pubkey'>> = {}
): NostrEvent {
  const sk = generateSecretKey();
  return finalizeEvent(
    {
      kind: 1,
      content: 'test content',
      tags: [],
      created_at: Math.floor(Date.now() / 1000),
      ...overrides,
    },
    sk
  );
}

/**
 * Mount the handler on a bare Hono app and dispatch a POST /write request.
 */
async function makeRequest(
  config: WriteHandlerConfig,
  body: unknown,
  headers: Record<string, string> = {},
  rawBody?: string
): Promise<Response> {
  const handler = createWriteHandler(config);
  const app = new Hono();
  app.post('/write', (c) => handler.handleWrite(c));

  const request = new Request('http://localhost/write', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      ...headers,
    },
    body: rawBody ?? JSON.stringify(body),
  });

  return app.fetch(request);
}

describe('Write handler', () => {
  it('stores a valid signed event and calls onStored once', async () => {
    // Given: an in-memory store, a tracking callback, and a signed event
    const eventStore = new InMemoryEventStore();
    const onStored = vi.fn();
    const event = createValidSignedEvent();

    // When: the write is submitted
    const response = await makeRequest(
      { eventStore, devMode: false, onStored },
      { event }
    );

    // Then: 200 with the event id (no payment fields -- nothing to echo)
    expect(response.status).toBe(200);
    const body = (await response.json()) as Record<string, unknown>;
    expect(body['eventId']).toBe(event.id);
    expect(body['payer']).toBeUndefined();
    expect(body['amount']).toBeUndefined();
    expect(body['chain']).toBeUndefined();
    expect(typeof body['storedAt']).toBe('number');

    // And: the event is present in the store
    const stored = eventStore.get(event.id);
    expect(stored).toBeDefined();
    expect(stored?.id).toBe(event.id);
    expect(stored?.sig).toBe(event.sig);

    // And: onStored was called exactly once with the stored event
    expect(onStored).toHaveBeenCalledOnce();
    expect(onStored.mock.calls[0]?.[0]?.id).toBe(event.id);
  });

  it('returns 400 for malformed JSON body', async () => {
    const eventStore = new InMemoryEventStore();
    const response = await makeRequest(
      { eventStore, devMode: false },
      undefined,
      {},
      'not valid json{{{'
    );

    expect(response.status).toBe(400);
    const body = (await response.json()) as Record<string, unknown>;
    expect(String(body['error'])).toMatch(/invalid/i);
  });

  it('returns 400 when the event field is missing', async () => {
    const eventStore = new InMemoryEventStore();
    const response = await makeRequest(
      { eventStore, devMode: false },
      {
        somethingElse: true,
      }
    );

    expect(response.status).toBe(400);
    const body = (await response.json()) as Record<string, unknown>;
    expect(String(body['error'])).toMatch(/event/i);
  });

  it('returns 422 for an invalid signature in non-dev mode', async () => {
    // Given: a validly-structured event with a tampered (invalid) signature
    const eventStore = new InMemoryEventStore();
    const onStored = vi.fn();
    const badEvent = { ...createValidSignedEvent(), sig: '0'.repeat(128) };

    // When: devMode is false
    const response = await makeRequest(
      { eventStore, devMode: false, onStored },
      { event: badEvent }
    );

    // Then: 422 and nothing is stored
    expect(response.status).toBe(422);
    const body = (await response.json()) as Record<string, unknown>;
    expect(String(body['error'])).toMatch(/signature/i);
    expect(eventStore.get(badEvent.id)).toBeUndefined();
    expect(onStored).not.toHaveBeenCalled();
  });

  it('accepts the SAME bad-signature event when devMode is true', async () => {
    // Given: the same tampered event used above
    const eventStore = new InMemoryEventStore();
    const onStored = vi.fn();
    const badEvent = { ...createValidSignedEvent(), sig: '0'.repeat(128) };

    // When: devMode is true (signature verification is skipped)
    const response = await makeRequest(
      { eventStore, devMode: true, onStored },
      { event: badEvent }
    );

    // Then: 200, stored, and onStored fired
    expect(response.status).toBe(200);
    const stored = eventStore.get(badEvent.id);
    expect(stored).toBeDefined();
    expect(stored?.id).toBe(badEvent.id);
    expect(stored?.sig).toBe(badEvent.sig);
    expect(onStored).toHaveBeenCalledOnce();
  });

  it('ignores retired X-TOON-* headers from a stale caller (no successor header, connector ADR 0036)', async () => {
    // Given: a valid signed event
    const eventStore = new InMemoryEventStore();
    const event = createValidSignedEvent();

    // When: the caller still sends the retired headers (e.g. a stale
    // terminator build)
    const response = await makeRequest(
      { eventStore, devMode: false },
      { event },
      {
        'X-TOON-Payer': '0xpayer',
        'X-TOON-Amount': '5500',
        'X-TOON-Chain': '31337',
      }
    );

    // Then: 200, stored, and the response echoes none of them
    expect(response.status).toBe(200);
    const body = (await response.json()) as Record<string, unknown>;
    expect(body['eventId']).toBe(event.id);
    expect(body['payer']).toBeUndefined();
    expect(body['amount']).toBeUndefined();
    expect(body['chain']).toBeUndefined();
    expect(eventStore.get(event.id)?.id).toBe(event.id);
  });

  it('broadcasts but never stores an ephemeral event (NIP-16, connector#685)', async () => {
    // Given: a signed ephemeral event (the huddle audio-frame kind)
    const eventStore = new InMemoryEventStore();
    const onStored = vi.fn();
    const event = createValidSignedEvent({ kind: 20001 });

    // When: it arrives on the paid-write surface
    const response = await makeRequest(
      { eventStore, devMode: false, onStored },
      { event }
    );

    // Then: 200 and the live-broadcast hook fired -- but nothing was
    // persisted, so REQ history will never serve it and the synchronous
    // per-event disk write is off the ephemeral hot path entirely.
    expect(response.status).toBe(200);
    const body = (await response.json()) as Record<string, unknown>;
    expect(body['eventId']).toBe(event.id);
    expect(onStored).toHaveBeenCalledTimes(1);
    // Compare against a symbol-free copy: since the paid-ephemeral verify
    // skip (relay#85) the handler no longer schnorr-verifies ephemeral
    // kinds, so the broadcast event carries no nostr-tools verifiedSymbol
    // stamp (an id check is not a signature verdict).
    expect(onStored).toHaveBeenCalledWith(JSON.parse(JSON.stringify(event)));
    expect(eventStore.get(event.id)).toBeUndefined();
  });

  describe('paid-ephemeral verify skip (relay#85)', () => {
    it('accepts an ephemeral event with an INVALID signature by default (id check only)', async () => {
      // Given: an ephemeral (huddle-frame kind) event whose sig is garbage
      // but whose id still matches the bytes. Payment is the admission gate
      // and clients verify signatures themselves -- the relay must not spend
      // schnorr time on it.
      const eventStore = new InMemoryEventStore();
      const onStored = vi.fn();
      const event = {
        ...createValidSignedEvent({ kind: 20001 }),
        sig: '0'.repeat(128),
      };

      const response = await makeRequest(
        { eventStore, devMode: false, onStored },
        { event }
      );

      // Then: 200, broadcast fired, nothing persisted (NIP-16)
      expect(response.status).toBe(200);
      expect(onStored).toHaveBeenCalledOnce();
      expect(eventStore.get(event.id)).toBeUndefined();
    });

    it('still rejects an ephemeral event whose id does not match its bytes (422)', async () => {
      // The SHA-256 id check is the integrity floor the skip path keeps:
      // broadcast bytes must never disagree with the id clients verify by.
      const eventStore = new InMemoryEventStore();
      const onStored = vi.fn();
      const event = {
        ...createValidSignedEvent({ kind: 20001 }),
        content: 'tampered after signing',
      };

      const response = await makeRequest(
        { eventStore, devMode: false, onStored },
        { event }
      );

      expect(response.status).toBe(422);
      const body = (await response.json()) as Record<string, unknown>;
      expect(String(body['error'])).toMatch(/id/i);
      expect(onStored).not.toHaveBeenCalled();
    });

    it('verifyEphemeral: true restores FULL schnorr verification on ephemeral kinds', async () => {
      // The community-operator escape hatch: same bad-sig ephemeral event,
      // full verification back on -> 422.
      const eventStore = new InMemoryEventStore();
      const onStored = vi.fn();
      const event = {
        ...createValidSignedEvent({ kind: 20001 }),
        sig: '0'.repeat(128),
      };

      const response = await makeRequest(
        { eventStore, devMode: false, verifyEphemeral: true, onStored },
        { event }
      );

      expect(response.status).toBe(422);
      const body = (await response.json()) as Record<string, unknown>;
      expect(String(body['error'])).toMatch(/signature/i);
      expect(onStored).not.toHaveBeenCalled();
    });

    it('never skips schnorr for persistent kinds (skip is scoped to 20000 <= kind < 30000)', async () => {
      // Kinds just outside the ephemeral range keep full verification even
      // with the skip at its default.
      for (const kind of [19999, 30000]) {
        const eventStore = new InMemoryEventStore();
        const event = {
          ...createValidSignedEvent({
            kind,
            tags: kind === 30000 ? [['d', 'x']] : [],
          }),
          sig: '0'.repeat(128),
        };
        const response = await makeRequest(
          { eventStore, devMode: false },
          { event }
        );
        expect(response.status, `kind ${kind} must still verify`).toBe(422);
        expect(eventStore.get(event.id)).toBeUndefined();
      }
    });
  });

  describe('async verify + per-session ordering (relay#85 worker pool)', () => {
    it('awaits an injected async verifier (200 on true, 422 on false)', async () => {
      const eventStore = new InMemoryEventStore();
      const event = createValidSignedEvent();

      const accept = await makeRequest(
        { eventStore, devMode: false, verifyEvent: async () => true },
        { event }
      );
      expect(accept.status).toBe(200);

      const reject = await makeRequest(
        {
          eventStore: new InMemoryEventStore(),
          devMode: false,
          verifyEvent: async () => false,
        },
        { event }
      );
      expect(reject.status).toBe(422);
    });

    it('sequential same-session writes are never reordered by async verify (connector-serialization contract)', async () => {
      // Ordering is enforced UPSTREAM: the connector serializes each BTP
      // session's POSTs -- it does not send request N+1 until response N
      // arrived. This test documents and guards the relay-side assumption:
      // with an adversarially slow-then-fast async verifier, sequentially
      // awaited requests still store and respond in submission order. (Only
      // CONCURRENT requests may complete out of order -- which the upstream
      // contract precludes within a session.)
      const eventStore = new InMemoryEventStore();
      const storedOrder: string[] = [];
      const handler = createWriteHandler({
        eventStore,
        devMode: false,
        // First event verifies slowest -- if the handler leaked concurrency
        // for sequential callers, later events would overtake it.
        verifyEvent: (event) =>
          new Promise<boolean>((resolve) => {
            const delay = event.content === 'seq-0' ? 30 : 1;
            setTimeout(() => resolve(true), delay);
          }),
        onStored: (event) => storedOrder.push(event.content),
      });
      const app = new Hono();
      app.post('/write', (c) => handler.handleWrite(c));

      const contents = ['seq-0', 'seq-1', 'seq-2', 'seq-3'];
      for (const content of contents) {
        const event = createValidSignedEvent({ content });
        const response = await app.fetch(
          new Request('http://localhost/write', {
            method: 'POST',
            headers: { 'Content-Type': 'application/json' },
            body: JSON.stringify({ event }),
          })
        );
        expect(response.status).toBe(200);
        const body = (await response.json()) as Record<string, unknown>;
        // Each response carries ITS request's event id.
        expect(body['eventId']).toBe(event.id);
      }

      expect(storedOrder).toEqual(contents);
    });
  });

  describe('per-write logging (relay#85)', () => {
    afterEach(() => {
      vi.restoreAllMocks();
    });

    it('does NOT log per write by default', async () => {
      // Given: a console spy and a default (logWrites unset) handler
      const logSpy = vi.spyOn(console, 'log').mockImplementation(() => {});
      const eventStore = new InMemoryEventStore();
      const event = createValidSignedEvent();

      // When: a write is accepted
      const response = await makeRequest(
        { eventStore, devMode: false },
        { event }
      );

      // Then: 200, and no per-write line hit the console
      expect(response.status).toBe(200);
      expect(logSpy).not.toHaveBeenCalled();
    });

    it('logs one line per write when logWrites is true', async () => {
      const logSpy = vi.spyOn(console, 'log').mockImplementation(() => {});
      const eventStore = new InMemoryEventStore();
      const event = createValidSignedEvent();

      const response = await makeRequest(
        { eventStore, devMode: false, logWrites: true },
        { event }
      );

      expect(response.status).toBe(200);
      expect(logSpy).toHaveBeenCalledOnce();
      const logLine = String(logSpy.mock.calls[0]?.[0]);
      expect(logLine).toContain(event.id);
      expect(logLine).not.toMatch(/payer|amount|chain/i);
    });
  });

  it('treats the whole NIP-16 ephemeral range as broadcast-only, and its edges as stored', async () => {
    const cases: { kind: number; stored: boolean }[] = [
      { kind: 19999, stored: true }, // last replaceable kind
      { kind: 20000, stored: false }, // first ephemeral kind
      { kind: 29999, stored: false }, // last ephemeral kind
      { kind: 30000, stored: true }, // first parameterized-replaceable kind
    ];
    for (const { kind, stored } of cases) {
      const eventStore = new InMemoryEventStore();
      const event = createValidSignedEvent({
        kind,
        // Parameterized-replaceable kinds key on the d tag.
        tags: kind === 30000 ? [['d', 'x']] : [],
      });
      const response = await makeRequest(
        { eventStore, devMode: false },
        { event }
      );
      expect(response.status).toBe(200);
      expect(
        eventStore.get(event.id) !== undefined,
        `kind ${kind} should ${stored ? '' : 'NOT '}be stored`
      ).toBe(stored);
    }
  });
});
