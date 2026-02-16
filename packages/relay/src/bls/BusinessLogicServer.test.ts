import { describe, it, expect, beforeEach, vi } from 'vitest';
import { createHash } from 'crypto';
import { generateSecretKey, finalizeEvent, getPublicKey } from 'nostr-tools/pure';
import type { NostrEvent } from 'nostr-tools/pure';
import { BusinessLogicServer, generateFulfillment } from './BusinessLogicServer.js';
import { ILP_ERROR_CODES, BlsError, isValidPubkey, SPSP_REQUEST_KIND } from './types.js';
import type { EventStore } from '../storage/index.js';
import { encodeEventToToon } from '../toon/index.js';
import { SqliteEventStore } from '../storage/index.js';
import { PricingService } from '../pricing/index.js';

/**
 * Helper to create a properly signed test event.
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
 * Helper to create base64-encoded TOON data from an event.
 */
function eventToBase64Toon(event: NostrEvent): string {
  const toonData = encodeEventToToon(event);
  return Buffer.from(toonData).toString('base64');
}

/**
 * Create a mock EventStore for unit testing.
 */
function createMockEventStore(): EventStore {
  return {
    store: vi.fn(),
    get: vi.fn(),
    query: vi.fn().mockReturnValue([]),
    close: vi.fn(),
  };
}

/**
 * Helper to create event from specific secret key.
 */
function createEventFromKey(
  sk: Uint8Array,
  overrides: Partial<Omit<NostrEvent, 'id' | 'sig' | 'pubkey'>> = {}
): NostrEvent {
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

describe('generateFulfillment', () => {
  it('should generate SHA-256 hash of event ID as base64', () => {
    const eventId = 'abc123';
    const expected = createHash('sha256').update(eventId).digest('base64');
    expect(generateFulfillment(eventId)).toBe(expected);
  });

  it('should produce consistent results for same input', () => {
    const eventId = 'test-event-id';
    const result1 = generateFulfillment(eventId);
    const result2 = generateFulfillment(eventId);
    expect(result1).toBe(result2);
  });

  it('should produce different results for different inputs', () => {
    const result1 = generateFulfillment('event1');
    const result2 = generateFulfillment('event2');
    expect(result1).not.toBe(result2);
  });
});

describe('BusinessLogicServer', () => {
  let bls: BusinessLogicServer;
  let mockEventStore: ReturnType<typeof createMockEventStore>;

  beforeEach(() => {
    mockEventStore = createMockEventStore();
    bls = new BusinessLogicServer({ basePricePerByte: 10n }, mockEventStore);
  });

  describe('POST /handle-packet', () => {
    it('should accept payment when amount meets price', async () => {
      const event = createValidSignedEvent();
      const base64Data = eventToBase64Toon(event);

      const response = await bls.getApp().request('/handle-packet', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          amount: '1000000', // Large enough for any event
          destination: 'g.agent.test',
          data: base64Data,
        }),
      });

      expect(response.status).toBe(200);
      const json = await response.json();
      expect(json.accept).toBe(true);
      expect(json.fulfillment).toBeDefined();
      expect(json.metadata?.eventId).toBe(event.id);
      expect(json.metadata?.storedAt).toBeTypeOf('number');
    });

    it('should store event in EventStore on success', async () => {
      const event = createValidSignedEvent();
      const base64Data = eventToBase64Toon(event);

      await bls.getApp().request('/handle-packet', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          amount: '1000000',
          destination: 'g.agent.test',
          data: base64Data,
        }),
      });

      expect(mockEventStore.store).toHaveBeenCalledTimes(1);
      const storedEvent = (mockEventStore.store as ReturnType<typeof vi.fn>)
        .mock.calls[0][0];
      expect(storedEvent.id).toBe(event.id);
    });

    it('should reject insufficient payment with F06', async () => {
      const event = createValidSignedEvent();
      const base64Data = eventToBase64Toon(event);

      const response = await bls.getApp().request('/handle-packet', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          amount: '1', // Too low
          destination: 'g.agent.test',
          data: base64Data,
        }),
      });

      expect(response.status).toBe(400);
      const json = await response.json();
      expect(json.accept).toBe(false);
      expect(json.code).toBe(ILP_ERROR_CODES.INSUFFICIENT_AMOUNT);
      expect(json.message).toBe('Insufficient payment amount');
      expect(json.metadata?.required).toBeDefined();
      expect(json.metadata?.received).toBe('1');
    });

    it('should NOT store event when payment is insufficient', async () => {
      const event = createValidSignedEvent();
      const base64Data = eventToBase64Toon(event);

      await bls.getApp().request('/handle-packet', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          amount: '1',
          destination: 'g.agent.test',
          data: base64Data,
        }),
      });

      expect(mockEventStore.store).not.toHaveBeenCalled();
    });

    it('should reject invalid base64 data with F00', async () => {
      const response = await bls.getApp().request('/handle-packet', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          amount: '1000000',
          destination: 'g.agent.test',
          data: '!!!not-valid-base64!!!',
        }),
      });

      expect(response.status).toBe(400);
      const json = await response.json();
      expect(json.accept).toBe(false);
      expect(json.code).toBe(ILP_ERROR_CODES.BAD_REQUEST);
    });

    it('should reject invalid TOON data with F00', async () => {
      // Valid base64 but not valid TOON
      const invalidToon = Buffer.from('not a toon encoded event').toString(
        'base64'
      );

      const response = await bls.getApp().request('/handle-packet', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          amount: '1000000',
          destination: 'g.agent.test',
          data: invalidToon,
        }),
      });

      expect(response.status).toBe(400);
      const json = await response.json();
      expect(json.accept).toBe(false);
      expect(json.code).toBe(ILP_ERROR_CODES.BAD_REQUEST);
      expect(json.message).toContain('Invalid TOON data');
    });

    it('should reject event with invalid signature with F00', async () => {
      // Create a valid event, then tamper with the content
      const event = createValidSignedEvent();
      // Tamper with content (signature is now invalid)
      const tamperedEvent = { ...event, content: 'tampered content' };
      const base64Data = eventToBase64Toon(tamperedEvent);

      const response = await bls.getApp().request('/handle-packet', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          amount: '1000000',
          destination: 'g.agent.test',
          data: base64Data,
        }),
      });

      expect(response.status).toBe(400);
      const json = await response.json();
      expect(json.accept).toBe(false);
      expect(json.code).toBe(ILP_ERROR_CODES.BAD_REQUEST);
      expect(json.message).toBe('Invalid event signature');
    });

    it('should NOT store event when signature is invalid', async () => {
      const event = createValidSignedEvent();
      const tamperedEvent = { ...event, content: 'tampered' };
      const base64Data = eventToBase64Toon(tamperedEvent);

      await bls.getApp().request('/handle-packet', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          amount: '1000000',
          destination: 'g.agent.test',
          data: base64Data,
        }),
      });

      expect(mockEventStore.store).not.toHaveBeenCalled();
    });

    it('should reject missing required fields with F00', async () => {
      const response = await bls.getApp().request('/handle-packet', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          amount: '1000',
          // missing destination and data
        }),
      });

      expect(response.status).toBe(400);
      const json = await response.json();
      expect(json.accept).toBe(false);
      expect(json.code).toBe(ILP_ERROR_CODES.BAD_REQUEST);
      expect(json.message).toContain('Missing required fields');
    });

    it('should generate correct fulfillment from event.id', async () => {
      const event = createValidSignedEvent();
      const base64Data = eventToBase64Toon(event);
      const expectedFulfillment = generateFulfillment(event.id);

      const response = await bls.getApp().request('/handle-packet', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          amount: '1000000',
          destination: 'g.agent.test',
          data: base64Data,
        }),
      });

      const json = await response.json();
      expect(json.fulfillment).toBe(expectedFulfillment);
    });

    it('should calculate price correctly based on TOON byte length', async () => {
      const event = createValidSignedEvent({ content: 'short' });
      const toonData = encodeEventToToon(event);
      const base64Data = Buffer.from(toonData).toString('base64');
      const expectedPrice = BigInt(toonData.length) * 10n;

      // Amount exactly meets price
      const response = await bls.getApp().request('/handle-packet', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          amount: expectedPrice.toString(),
          destination: 'g.agent.test',
          data: base64Data,
        }),
      });

      const json = await response.json();
      expect(json.accept).toBe(true);
    });

    it('should reject when amount is exactly one less than price', async () => {
      const event = createValidSignedEvent({ content: 'test' });
      const toonData = encodeEventToToon(event);
      const base64Data = Buffer.from(toonData).toString('base64');
      const price = BigInt(toonData.length) * 10n;
      const insufficientAmount = price - 1n;

      const response = await bls.getApp().request('/handle-packet', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          amount: insufficientAmount.toString(),
          destination: 'g.agent.test',
          data: base64Data,
        }),
      });

      const json = await response.json();
      expect(json.accept).toBe(false);
      expect(json.code).toBe(ILP_ERROR_CODES.INSUFFICIENT_AMOUNT);
    });
  });

  describe('GET /health', () => {
    it('should return healthy status', async () => {
      const response = await bls.getApp().request('/health', {
        method: 'GET',
      });

      expect(response.status).toBe(200);
      const json = await response.json();
      expect(json.status).toBe('healthy');
      expect(json.timestamp).toBeTypeOf('number');
    });

    it('should return current timestamp', async () => {
      const before = Date.now();
      const response = await bls.getApp().request('/health', {
        method: 'GET',
      });
      const after = Date.now();

      const json = await response.json();
      expect(json.timestamp).toBeGreaterThanOrEqual(before);
      expect(json.timestamp).toBeLessThanOrEqual(after);
    });
  });
});

describe('BusinessLogicServer integration', () => {
  let bls: BusinessLogicServer;
  let eventStore: SqliteEventStore;

  beforeEach(() => {
    eventStore = new SqliteEventStore(':memory:');
    bls = new BusinessLogicServer({ basePricePerByte: 10n }, eventStore);
  });

  it('should store and retrieve event after successful payment', async () => {
    const event = createValidSignedEvent({ content: 'integration test' });
    const base64Data = eventToBase64Toon(event);

    const response = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: '1000000',
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(response.status).toBe(200);

    // Verify event was stored
    const storedEvent = eventStore.get(event.id);
    expect(storedEvent).toBeDefined();
    expect(storedEvent?.id).toBe(event.id);
    expect(storedEvent?.content).toBe('integration test');
  });

  it('should handle full flow with real signed events', async () => {
    // Create multiple events
    const events = [
      createValidSignedEvent({ content: 'event 1' }),
      createValidSignedEvent({ content: 'event 2' }),
      createValidSignedEvent({ content: 'event 3' }),
    ];

    for (const event of events) {
      const base64Data = eventToBase64Toon(event);
      const response = await bls.getApp().request('/handle-packet', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          amount: '1000000',
          destination: 'g.agent.test',
          data: base64Data,
        }),
      });
      expect(response.status).toBe(200);
    }

    // Verify all events are retrievable
    for (const event of events) {
      const stored = eventStore.get(event.id);
      expect(stored?.id).toBe(event.id);
    }
  });

  it('should not store event when payment fails', async () => {
    const event = createValidSignedEvent();
    const base64Data = eventToBase64Toon(event);

    await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: '1', // Too low
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    // Verify event was NOT stored
    const storedEvent = eventStore.get(event.id);
    expect(storedEvent).toBeUndefined();
  });

  it('should query stored events by filters', async () => {
    const sk = generateSecretKey();
    const event1 = finalizeEvent(
      {
        kind: 1,
        content: 'note 1',
        tags: [],
        created_at: Math.floor(Date.now() / 1000),
      },
      sk
    );
    const event2 = finalizeEvent(
      {
        kind: 1,
        content: 'note 2',
        tags: [],
        created_at: Math.floor(Date.now() / 1000) + 1,
      },
      sk
    );

    // Store both events via payment
    for (const event of [event1, event2]) {
      const base64Data = eventToBase64Toon(event);
      await bls.getApp().request('/handle-packet', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          amount: '1000000',
          destination: 'g.agent.test',
          data: base64Data,
        }),
      });
    }

    // Query by author
    const results = eventStore.query([{ authors: [event1.pubkey] }]);
    expect(results.length).toBe(2);
  });
});

describe('BusinessLogicServer with PricingService', () => {
  let bls: BusinessLogicServer;
  let mockEventStore: ReturnType<typeof createMockEventStore>;
  let pricingService: PricingService;

  beforeEach(() => {
    mockEventStore = createMockEventStore();
    pricingService = new PricingService({
      basePricePerByte: 10n,
      kindOverrides: new Map([
        [0, 0n], // Free profiles
        [30023, 100n], // Expensive articles
        [1, 5n], // Cheaper notes
      ]),
    });
    bls = new BusinessLogicServer(
      { basePricePerByte: 10n, pricingService },
      mockEventStore
    );
  });

  it('should use kind override pricing for kind 0 (free)', async () => {
    const event = createValidSignedEvent({ kind: 0, content: '{"name":"test"}' });
    const base64Data = eventToBase64Toon(event);

    // Even with amount 0, it should accept (free profile)
    const response = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: '0',
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(response.status).toBe(200);
    const json = await response.json();
    expect(json.accept).toBe(true);
  });

  it('should use kind override pricing for kind 30023 (expensive)', async () => {
    const event = createValidSignedEvent({ kind: 30023, content: 'article' });
    const toonData = encodeEventToToon(event);
    const base64Data = Buffer.from(toonData).toString('base64');
    const expectedPrice = BigInt(toonData.length) * 100n; // 100 per byte for kind 30023

    // Pay exactly the required price
    const response = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: expectedPrice.toString(),
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(response.status).toBe(200);
    const json = await response.json();
    expect(json.accept).toBe(true);
  });

  it('should reject insufficient payment for kind 30023', async () => {
    const event = createValidSignedEvent({ kind: 30023, content: 'article' });
    const toonData = encodeEventToToon(event);
    const base64Data = Buffer.from(toonData).toString('base64');
    // Pay base rate (10n) instead of override rate (100n)
    const insufficientPrice = BigInt(toonData.length) * 10n;

    const response = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: insufficientPrice.toString(),
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(response.status).toBe(400);
    const json = await response.json();
    expect(json.accept).toBe(false);
    expect(json.code).toBe(ILP_ERROR_CODES.INSUFFICIENT_AMOUNT);
  });

  it('should use kind override pricing for kind 1 (cheaper)', async () => {
    const event = createValidSignedEvent({ kind: 1, content: 'note' });
    const toonData = encodeEventToToon(event);
    const base64Data = Buffer.from(toonData).toString('base64');
    const expectedPrice = BigInt(toonData.length) * 5n; // 5 per byte for kind 1

    const response = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: expectedPrice.toString(),
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(response.status).toBe(200);
    const json = await response.json();
    expect(json.accept).toBe(true);
  });

  it('should fall back to base price for unlisted kinds', async () => {
    const event = createValidSignedEvent({ kind: 7, content: '+' }); // Reaction, not in overrides
    const toonData = encodeEventToToon(event);
    const base64Data = Buffer.from(toonData).toString('base64');
    const expectedPrice = BigInt(toonData.length) * 10n; // Base price

    const response = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: expectedPrice.toString(),
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(response.status).toBe(200);
    const json = await response.json();
    expect(json.accept).toBe(true);
  });

  it('should reject payment below base price for unlisted kinds', async () => {
    const event = createValidSignedEvent({ kind: 7, content: '+' });
    const toonData = encodeEventToToon(event);
    const base64Data = Buffer.from(toonData).toString('base64');
    const insufficientPrice = BigInt(toonData.length) * 5n; // Less than base 10n

    const response = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: insufficientPrice.toString(),
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(response.status).toBe(400);
    const json = await response.json();
    expect(json.accept).toBe(false);
    expect(json.code).toBe(ILP_ERROR_CODES.INSUFFICIENT_AMOUNT);
  });
});

describe('BusinessLogicServer backwards compatibility', () => {
  it('should work without PricingService (legacy behavior)', async () => {
    const mockEventStore = createMockEventStore();
    // No pricingService provided - should use simple calculation
    const bls = new BusinessLogicServer(
      { basePricePerByte: 10n },
      mockEventStore
    );

    const event = createValidSignedEvent({ kind: 0, content: 'profile' });
    const toonData = encodeEventToToon(event);
    const base64Data = Buffer.from(toonData).toString('base64');
    const expectedPrice = BigInt(toonData.length) * 10n;

    const response = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: expectedPrice.toString(),
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(response.status).toBe(200);
    const json = await response.json();
    expect(json.accept).toBe(true);
  });
});

describe('isValidPubkey', () => {
  it('should return true for valid 64-character lowercase hex pubkey', () => {
    const validPubkey = '3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d';
    expect(isValidPubkey(validPubkey)).toBe(true);
  });

  it('should return false for uppercase hex characters', () => {
    const upperPubkey = '3BF0C63FCB93463407AF97A5E5EE64FA883D107EF9E558472C4EB9AAAEFA459D';
    expect(isValidPubkey(upperPubkey)).toBe(false);
  });

  it('should return false for pubkey with less than 64 characters', () => {
    const shortPubkey = '3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459';
    expect(isValidPubkey(shortPubkey)).toBe(false);
  });

  it('should return false for pubkey with more than 64 characters', () => {
    const longPubkey = '3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d0';
    expect(isValidPubkey(longPubkey)).toBe(false);
  });

  it('should return false for pubkey with non-hex characters', () => {
    const invalidPubkey = '3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459g';
    expect(isValidPubkey(invalidPubkey)).toBe(false);
  });

  it('should return false for empty string', () => {
    expect(isValidPubkey('')).toBe(false);
  });
});

describe('BusinessLogicServer with ownerPubkey', () => {
  let ownerSk: Uint8Array;
  let ownerPubkey: string;
  let mockEventStore: ReturnType<typeof createMockEventStore>;
  let bls: BusinessLogicServer;

  beforeEach(() => {
    ownerSk = generateSecretKey();
    ownerPubkey = getPublicKey(ownerSk);
    mockEventStore = createMockEventStore();
    bls = new BusinessLogicServer(
      { basePricePerByte: 10n, ownerPubkey },
      mockEventStore
    );
  });

  it('should accept owner events with zero payment', async () => {
    const event = createEventFromKey(ownerSk);
    const base64Data = eventToBase64Toon(event);

    const response = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: '0',
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(response.status).toBe(200);
    const json = await response.json();
    expect(json.accept).toBe(true);
    expect(json.fulfillment).toBeDefined();
    expect(json.metadata?.eventId).toBe(event.id);
  });

  it('should store owner events successfully', async () => {
    const event = createEventFromKey(ownerSk);
    const base64Data = eventToBase64Toon(event);

    await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: '0',
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(mockEventStore.store).toHaveBeenCalledTimes(1);
    const storedEvent = (mockEventStore.store as ReturnType<typeof vi.fn>)
      .mock.calls[0][0];
    expect(storedEvent.id).toBe(event.id);
  });

  it('should still require valid signature for owner events', async () => {
    const event = createEventFromKey(ownerSk);
    // Tamper with content (signature is now invalid)
    const tamperedEvent = { ...event, content: 'tampered content' };
    const base64Data = eventToBase64Toon(tamperedEvent);

    const response = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: '0',
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(response.status).toBe(400);
    const json = await response.json();
    expect(json.accept).toBe(false);
    expect(json.code).toBe(ILP_ERROR_CODES.BAD_REQUEST);
    expect(json.message).toBe('Invalid event signature');
    expect(mockEventStore.store).not.toHaveBeenCalled();
  });

  it('should reject non-owner events with zero payment', async () => {
    const nonOwnerSk = generateSecretKey();
    const event = createEventFromKey(nonOwnerSk);
    const base64Data = eventToBase64Toon(event);

    const response = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: '0',
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(response.status).toBe(400);
    const json = await response.json();
    expect(json.accept).toBe(false);
    expect(json.code).toBe(ILP_ERROR_CODES.INSUFFICIENT_AMOUNT);
    expect(mockEventStore.store).not.toHaveBeenCalled();
  });

  it('should accept non-owner events with sufficient payment', async () => {
    const nonOwnerSk = generateSecretKey();
    const event = createEventFromKey(nonOwnerSk);
    const base64Data = eventToBase64Toon(event);

    const response = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: '1000000',
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(response.status).toBe(200);
    const json = await response.json();
    expect(json.accept).toBe(true);
    expect(mockEventStore.store).toHaveBeenCalledTimes(1);
  });

  it('should require payment for all events when ownerPubkey is not configured', async () => {
    const blsWithoutOwner = new BusinessLogicServer(
      { basePricePerByte: 10n },
      mockEventStore
    );
    const event = createEventFromKey(ownerSk);
    const base64Data = eventToBase64Toon(event);

    const response = await blsWithoutOwner.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: '0',
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(response.status).toBe(400);
    const json = await response.json();
    expect(json.accept).toBe(false);
    expect(json.code).toBe(ILP_ERROR_CODES.INSUFFICIENT_AMOUNT);
  });

  it('should throw BlsError for invalid ownerPubkey format (too short)', () => {
    expect(() => {
      new BusinessLogicServer(
        { basePricePerByte: 10n, ownerPubkey: 'abc123' },
        mockEventStore
      );
    }).toThrow(BlsError);
  });

  it('should throw BlsError for invalid ownerPubkey format (uppercase)', () => {
    expect(() => {
      new BusinessLogicServer(
        { basePricePerByte: 10n, ownerPubkey: '3BF0C63FCB93463407AF97A5E5EE64FA883D107EF9E558472C4EB9AAAEFA459D' },
        mockEventStore
      );
    }).toThrow(BlsError);
  });

  it('should throw BlsError for invalid ownerPubkey format (non-hex)', () => {
    expect(() => {
      new BusinessLogicServer(
        { basePricePerByte: 10n, ownerPubkey: 'zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz' },
        mockEventStore
      );
    }).toThrow(BlsError);
  });

  it('should include correct error message for invalid ownerPubkey', () => {
    expect(() => {
      new BusinessLogicServer(
        { basePricePerByte: 10n, ownerPubkey: 'invalid' },
        mockEventStore
      );
    }).toThrow('Invalid ownerPubkey format: must be 64 lowercase hex characters');
  });
});

describe('BusinessLogicServer with ownerPubkey integration', () => {
  let ownerSk: Uint8Array;
  let ownerPubkey: string;
  let eventStore: SqliteEventStore;
  let bls: BusinessLogicServer;

  beforeEach(() => {
    ownerSk = generateSecretKey();
    ownerPubkey = getPublicKey(ownerSk);
    eventStore = new SqliteEventStore(':memory:');
    bls = new BusinessLogicServer(
      { basePricePerByte: 10n, ownerPubkey },
      eventStore
    );
  });

  it('should store and retrieve owner event after bypass', async () => {
    const event = createEventFromKey(ownerSk, { content: 'owner integration test' });
    const base64Data = eventToBase64Toon(event);

    const response = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: '0',
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(response.status).toBe(200);

    // Verify event was stored
    const storedEvent = eventStore.get(event.id);
    expect(storedEvent).toBeDefined();
    expect(storedEvent?.id).toBe(event.id);
    expect(storedEvent?.content).toBe('owner integration test');
  });

  it('should handle mix of owner and non-owner events in sequence', async () => {
    const nonOwnerSk = generateSecretKey();

    // Owner event (free)
    const ownerEvent = createEventFromKey(ownerSk, { content: 'owner event' });
    const ownerBase64 = eventToBase64Toon(ownerEvent);

    // Non-owner event (requires payment)
    const nonOwnerEvent = createEventFromKey(nonOwnerSk, { content: 'non-owner event' });
    const nonOwnerBase64 = eventToBase64Toon(nonOwnerEvent);

    // Submit owner event with amount=0
    const ownerResponse = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: '0',
        destination: 'g.agent.test',
        data: ownerBase64,
      }),
    });
    expect(ownerResponse.status).toBe(200);

    // Submit non-owner event with amount=0 (should fail)
    const nonOwnerFailResponse = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: '0',
        destination: 'g.agent.test',
        data: nonOwnerBase64,
      }),
    });
    expect(nonOwnerFailResponse.status).toBe(400);

    // Submit non-owner event with sufficient payment
    const nonOwnerSuccessResponse = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: '1000000',
        destination: 'g.agent.test',
        data: nonOwnerBase64,
      }),
    });
    expect(nonOwnerSuccessResponse.status).toBe(200);

    // Verify both events are stored
    expect(eventStore.get(ownerEvent.id)).toBeDefined();
    expect(eventStore.get(nonOwnerEvent.id)).toBeDefined();
  });

  it('should allow owner to store multiple events without payment', async () => {
    const events = [
      createEventFromKey(ownerSk, { content: 'event 1' }),
      createEventFromKey(ownerSk, { content: 'event 2' }),
      createEventFromKey(ownerSk, { content: 'event 3' }),
    ];

    for (const event of events) {
      const base64Data = eventToBase64Toon(event);
      const response = await bls.getApp().request('/handle-packet', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          amount: '0',
          destination: 'g.agent.test',
          data: base64Data,
        }),
      });
      expect(response.status).toBe(200);
    }

    // Verify all events are retrievable
    for (const event of events) {
      const stored = eventStore.get(event.id);
      expect(stored?.id).toBe(event.id);
    }
  });
});

describe('BusinessLogicServer with spspMinPrice', () => {
  let mockEventStore: ReturnType<typeof createMockEventStore>;

  beforeEach(() => {
    mockEventStore = createMockEventStore();
  });

  it('should accept kind:23194 events with amount=0 when spspMinPrice is 0n', async () => {
    const bls = new BusinessLogicServer(
      { basePricePerByte: 10n, spspMinPrice: 0n },
      mockEventStore
    );
    const event = createValidSignedEvent({ kind: SPSP_REQUEST_KIND, content: 'encrypted-spsp-request' });
    const base64Data = eventToBase64Toon(event);

    const response = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: '0',
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(response.status).toBe(200);
    const json = await response.json();
    expect(json.accept).toBe(true);
  });

  it('should still require standard payment for non-SPSP events when spspMinPrice is 0n', async () => {
    const bls = new BusinessLogicServer(
      { basePricePerByte: 10n, spspMinPrice: 0n },
      mockEventStore
    );
    const event = createValidSignedEvent({ kind: 1, content: 'regular note' });
    const base64Data = eventToBase64Toon(event);

    const response = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: '0',
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(response.status).toBe(400);
    const json = await response.json();
    expect(json.accept).toBe(false);
    expect(json.code).toBe(ILP_ERROR_CODES.INSUFFICIENT_AMOUNT);
  });

  it('should require standard payment for kind:23194 when spspMinPrice is not set', async () => {
    const bls = new BusinessLogicServer(
      { basePricePerByte: 10n },
      mockEventStore
    );
    const event = createValidSignedEvent({ kind: SPSP_REQUEST_KIND, content: 'encrypted-spsp-request' });
    const base64Data = eventToBase64Toon(event);

    const response = await bls.getApp().request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        amount: '0',
        destination: 'g.agent.test',
        data: base64Data,
      }),
    });

    expect(response.status).toBe(400);
    const json = await response.json();
    expect(json.accept).toBe(false);
    expect(json.code).toBe(ILP_ERROR_CODES.INSUFFICIENT_AMOUNT);
  });
});
