/**
 * Unit tests for the free ephemeral write handler (relay#129).
 *
 * POST /write-ephemeral has no payment gate, so these tests cover its own
 * admission controls in place of a claim check:
 *
 * - only ephemeral kinds (20000-29999) are accepted; everything else -> 400
 * - schnorr verification is ALWAYS full -- a bad signature on an ephemeral
 *   kind is rejected here, unlike the paid handler's default skip
 * - never stores (no EventStore dependency at all); onBroadcast fires once
 * - rate limit -> 429 once a key is over budget
 * - body size cap -> 413 (both a lying Content-Length and an honest oversized
 *   body)
 * - malformed / missing body -> 400
 */

import { describe, it, expect, vi } from 'vitest';
import { Hono } from 'hono';
import { generateSecretKey, finalizeEvent } from 'nostr-tools/pure';
import type { NostrEvent } from 'nostr-tools/pure';
import {
  createEphemeralWriteHandler,
  defaultClientKey,
  DEFAULT_EPHEMERAL_RATE_LIMIT,
  DEFAULT_EPHEMERAL_MAX_BODY_BYTES,
} from './write-ephemeral-handler.js';
import type { EphemeralWriteHandlerConfig } from './write-ephemeral-handler.js';
import { createRateLimiter } from '../rate-limiter.js';

function createValidSignedEvent(
  overrides: Partial<Omit<NostrEvent, 'id' | 'sig' | 'pubkey'>> = {}
): NostrEvent {
  const sk = generateSecretKey();
  return finalizeEvent(
    {
      kind: 20001,
      content: 'presence',
      tags: [],
      created_at: Math.floor(Date.now() / 1000),
      ...overrides,
    },
    sk
  );
}

async function makeRequest(
  config: EphemeralWriteHandlerConfig,
  body: unknown,
  headers: Record<string, string> = {},
  rawBody?: string
): Promise<Response> {
  const handler = createEphemeralWriteHandler(config);
  const app = new Hono();
  app.post('/write-ephemeral', (c) => handler.handleWrite(c));

  const request = new Request('http://localhost/write-ephemeral', {
    method: 'POST',
    headers: {
      'Content-Type': 'application/json',
      ...headers,
    },
    body: rawBody ?? JSON.stringify(body),
  });

  return app.fetch(request);
}

describe('Ephemeral write handler (relay#129)', () => {
  it('broadcasts a valid signed ephemeral event and never stores it', async () => {
    const onBroadcast = vi.fn();
    const event = createValidSignedEvent({ kind: 20002 });

    const response = await makeRequest({ onBroadcast }, { event });

    expect(response.status).toBe(200);
    const body = (await response.json()) as Record<string, unknown>;
    expect(body['eventId']).toBe(event.id);
    expect(typeof body['broadcastAt']).toBe('number');
    expect(onBroadcast).toHaveBeenCalledOnce();
    expect(onBroadcast.mock.calls[0]?.[0]?.id).toBe(event.id);
  });

  it('accepts the whole NIP-16 ephemeral range and rejects both edges', async () => {
    const cases: { kind: number; accepted: boolean }[] = [
      { kind: 19999, accepted: false }, // last replaceable kind
      { kind: 20000, accepted: true }, // first ephemeral kind
      { kind: 29999, accepted: true }, // last ephemeral kind
      { kind: 30000, accepted: false }, // first parameterized-replaceable kind
      { kind: 1, accepted: false }, // ordinary text note
    ];
    for (const { kind, accepted } of cases) {
      const onBroadcast = vi.fn();
      const event = createValidSignedEvent({
        kind,
        tags: kind === 30000 ? [['d', 'x']] : [],
      });
      const response = await makeRequest({ onBroadcast }, { event });

      if (accepted) {
        expect(response.status, `kind ${kind} should be accepted`).toBe(200);
        expect(onBroadcast).toHaveBeenCalledOnce();
      } else {
        expect(response.status, `kind ${kind} should be rejected`).toBe(400);
        expect(onBroadcast).not.toHaveBeenCalled();
      }
    }
  });

  it('rejects a persistent-kind event (400) without spending a verify call', async () => {
    const verifyEvent = vi.fn(async () => true);
    const event = createValidSignedEvent({ kind: 9 });

    const response = await makeRequest({ verifyEvent }, { event });

    expect(response.status).toBe(400);
    const body = (await response.json()) as Record<string, unknown>;
    expect(String(body['error'])).toMatch(/ephemeral/i);
    expect(verifyEvent).not.toHaveBeenCalled();
  });

  it('rejects an ephemeral event with an invalid signature (422) -- verify is ALWAYS on', async () => {
    const onBroadcast = vi.fn();
    const badEvent = {
      ...createValidSignedEvent({ kind: 20001 }),
      sig: '0'.repeat(128),
    };

    const response = await makeRequest({ onBroadcast }, { event: badEvent });

    expect(response.status).toBe(422);
    const body = (await response.json()) as Record<string, unknown>;
    expect(String(body['error'])).toMatch(/signature/i);
    expect(onBroadcast).not.toHaveBeenCalled();
  });

  it('has no config knob to skip verification (no devMode option exists on this handler)', async () => {
    // Structural guard: EphemeralWriteHandlerConfig simply has no devMode
    // field, so nothing a caller passes can weaken this. Assert behaviour
    // directly: a garbage config object with an extraneous devMode-shaped
    // property still gets full verification.
    const badEvent = {
      ...createValidSignedEvent({ kind: 20001 }),
      sig: '0'.repeat(128),
    };
    const response = await makeRequest(
      { devMode: true } as unknown as EphemeralWriteHandlerConfig,
      { event: badEvent }
    );
    expect(response.status).toBe(422);
  });

  it('returns 400 for malformed JSON body', async () => {
    const response = await makeRequest({}, undefined, {}, 'not valid json{{{');

    expect(response.status).toBe(400);
    const body = (await response.json()) as Record<string, unknown>;
    expect(String(body['error'])).toMatch(/invalid/i);
  });

  it('returns 400 when the event field is missing', async () => {
    const response = await makeRequest({}, { somethingElse: true });

    expect(response.status).toBe(400);
    const body = (await response.json()) as Record<string, unknown>;
    expect(String(body['error'])).toMatch(/event/i);
  });

  describe('rate limit bounds (relay#129)', () => {
    it('allows requests under budget and rejects the one that exceeds it (429)', async () => {
      let tick = 0;
      const rateLimiter = createRateLimiter({
        maxRequests: 2,
        windowMs: 10_000,
        now: () => tick,
      });

      for (let i = 0; i < 2; i++) {
        const event = createValidSignedEvent();
        const response = await makeRequest({ rateLimiter }, { event });
        expect(response.status, `request ${i} should be under budget`).toBe(
          200
        );
        tick += 1;
      }

      const overBudgetEvent = createValidSignedEvent();
      const response = await makeRequest(
        { rateLimiter },
        { event: overBudgetEvent }
      );
      expect(response.status).toBe(429);
      const body = (await response.json()) as Record<string, unknown>;
      expect(String(body['error'])).toMatch(/rate limit/i);
    });

    it('keys independently per getClientKey result', async () => {
      const tick = 0;
      const rateLimiter = createRateLimiter({
        maxRequests: 1,
        windowMs: 10_000,
        now: () => tick,
      });
      let key = 'client-a';
      const getClientKey = () => key;

      const first = await makeRequest(
        { rateLimiter, getClientKey },
        { event: createValidSignedEvent() }
      );
      expect(first.status).toBe(200);

      // Same key again, same tick -- over budget.
      const second = await makeRequest(
        { rateLimiter, getClientKey },
        { event: createValidSignedEvent() }
      );
      expect(second.status).toBe(429);

      // A different key has its own budget.
      key = 'client-b';
      const third = await makeRequest(
        { rateLimiter, getClientKey },
        { event: createValidSignedEvent() }
      );
      expect(third.status).toBe(200);
    });

    it('defaultClientKey falls back to a shared bucket when connection info is unavailable', async () => {
      // Bare Hono context with no real socket (as in every test in this
      // file, which dispatches via app.fetch() directly) -- getConnInfo
      // throws, and the handler must not propagate that.
      const app = new Hono();
      let observed: string | undefined;
      app.get('/x', (c) => {
        observed = defaultClientKey(c);
        return c.text('ok');
      });
      await app.fetch(new Request('http://localhost/x'));
      expect(observed).toBe('unknown');
    });

    it('exposes conservative documented defaults', () => {
      expect(DEFAULT_EPHEMERAL_RATE_LIMIT.maxRequests).toBeGreaterThan(0);
      expect(DEFAULT_EPHEMERAL_RATE_LIMIT.windowMs).toBeGreaterThan(0);
      expect(DEFAULT_EPHEMERAL_MAX_BODY_BYTES).toBeGreaterThan(0);
    });
  });

  describe('body size cap (relay#129)', () => {
    it('rejects a request whose Content-Length exceeds maxBodyBytes (413) without reading the body', async () => {
      const event = createValidSignedEvent();
      const oversized = JSON.stringify({ event, padding: 'x'.repeat(1000) });

      const response = await makeRequest(
        { maxBodyBytes: 16 },
        { event },
        { 'Content-Length': String(oversized.length) },
        oversized
      );

      expect(response.status).toBe(413);
    });

    it('rejects an oversized body even with no/lying Content-Length (413)', async () => {
      const event = createValidSignedEvent();
      const oversized = JSON.stringify({ event, padding: 'x'.repeat(1000) });

      const response = await makeRequest(
        { maxBodyBytes: 16 },
        { event },
        {},
        oversized
      );

      expect(response.status).toBe(413);
    });

    it('accepts a body within the cap', async () => {
      const event = createValidSignedEvent();
      const response = await makeRequest(
        { maxBodyBytes: DEFAULT_EPHEMERAL_MAX_BODY_BYTES },
        { event }
      );
      expect(response.status).toBe(200);
    });
  });

  describe('per-write logging (relay#129, matches write-handler.ts relay#85)', () => {
    it('does NOT log per write by default', async () => {
      const logSpy = vi.spyOn(console, 'log').mockImplementation(() => {});
      const event = createValidSignedEvent();

      const response = await makeRequest({}, { event });

      expect(response.status).toBe(200);
      expect(logSpy).not.toHaveBeenCalled();
      logSpy.mockRestore();
    });

    it('logs one line per write when logWrites is true', async () => {
      const logSpy = vi.spyOn(console, 'log').mockImplementation(() => {});
      const event = createValidSignedEvent();

      const response = await makeRequest({ logWrites: true }, { event });

      expect(response.status).toBe(200);
      expect(logSpy).toHaveBeenCalledOnce();
      const logLine = String(logSpy.mock.calls[0]?.[0]);
      expect(logLine).toContain(event.id);
      expect(logLine).toContain('write-ephemeral');
      logSpy.mockRestore();
    });
  });

  describe('async verify (matches write-handler.ts relay#85 worker pool)', () => {
    it('awaits an injected async verifier (200 on true, 422 on false)', async () => {
      const accept = await makeRequest(
        { verifyEvent: async () => true },
        { event: createValidSignedEvent() }
      );
      expect(accept.status).toBe(200);

      const reject = await makeRequest(
        { verifyEvent: async () => false },
        { event: createValidSignedEvent() }
      );
      expect(reject.status).toBe(422);
    });
  });
});
