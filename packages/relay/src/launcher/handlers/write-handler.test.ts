/**
 * Unit tests for the write handler.
 *
 * The handler accepts an event-as-JSON, trusts (does not validate) injected
 * X-TOON-* payment headers, verifies only the event signature for integrity,
 * and stores the event. These tests cover:
 *
 * - valid signed event + all three X-TOON headers -> 200, event stored,
 *   headers echoed, onStored called exactly once
 * - malformed / missing body -> 400
 * - invalid signature (non-dev) -> 422; same bad event with devMode -> 200
 * - headers absent -> still 200 (trusted-but-optional)
 */

import { describe, it, expect, vi } from 'vitest';
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
  it('stores a valid signed event, echoes headers, and calls onStored once', async () => {
    // Given: an in-memory store, a tracking callback, and a signed event
    const eventStore = new InMemoryEventStore();
    const onStored = vi.fn();
    const event = createValidSignedEvent();

    // When: the request carries all three trusted X-TOON headers
    const response = await makeRequest(
      { eventStore, devMode: false, onStored },
      { event },
      {
        'X-TOON-Payer': '0xpayer',
        'X-TOON-Amount': '5500',
        'X-TOON-Chain': '31337',
      }
    );

    // Then: 200 with the event id and echoed headers
    expect(response.status).toBe(200);
    const body = (await response.json()) as Record<string, unknown>;
    expect(body['eventId']).toBe(event.id);
    expect(body['payer']).toBe('0xpayer');
    expect(body['amount']).toBe('5500');
    expect(body['chain']).toBe('31337');
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

  it('still returns 200 when the X-TOON headers are absent (trusted-but-optional)', async () => {
    // Given: a valid signed event and no payment headers
    const eventStore = new InMemoryEventStore();
    const event = createValidSignedEvent();

    // When: no X-TOON-* headers are sent
    const response = await makeRequest(
      { eventStore, devMode: false },
      {
        event,
      }
    );

    // Then: 200, stored, and the echoed header fields are absent/undefined
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
    expect(onStored).toHaveBeenCalledWith(event);
    expect(eventStore.get(event.id)).toBeUndefined();
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
