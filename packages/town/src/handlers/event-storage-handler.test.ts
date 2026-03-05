/**
 * Tests for Story 2.1: Relay Event Storage Handler
 *
 * RED PHASE: All tests use describe.skip() because they import from
 * @crosstown/sdk which does not exist yet. The SDK will expose an
 * EventStorageHandler that reimplements the BLS handlePacket logic
 * as a composable handler with a context-based API (ctx.decode(),
 * ctx.accept(), ctx.reject()).
 *
 * Infrastructure: Real TOON codec, real SQLite :memory:, real nostr-tools
 * signatures. Only the connector transport is mocked.
 */

import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import {
  generateSecretKey,
  getPublicKey,
  finalizeEvent,
} from 'nostr-tools/pure';
import type { NostrEvent } from 'nostr-tools/pure';
import {
  encodeEventToToon,
  decodeEventFromToon,
  SqliteEventStore,
} from '@crosstown/relay';

// --- RED PHASE: These imports will fail because @crosstown/sdk does not exist yet ---
// The SDK handler replaces the BLS HTTP endpoint with a composable function
// that receives a handler context (ctx) with decode(), accept(), reject() methods.
import { createEventStorageHandler } from '@crosstown/sdk';

// ============================================================================
// Test Factories
// ============================================================================

/**
 * Create a properly signed Nostr event for testing.
 * Uses real nostr-tools signing (not mocked).
 */
/** Deterministic timestamp for reproducible tests (2026-01-01T00:00:00Z) */
const TEST_CREATED_AT = 1767225600;

function createValidSignedEvent(
  overrides: Partial<Omit<NostrEvent, 'id' | 'sig' | 'pubkey'>> = {},
  secretKey?: Uint8Array
): NostrEvent {
  const sk = secretKey ?? generateSecretKey();
  return finalizeEvent(
    {
      kind: 1,
      content: 'test content',
      tags: [],
      created_at: TEST_CREATED_AT,
      ...overrides,
    },
    sk
  );
}

/**
 * Encode a NostrEvent to base64-encoded TOON (the wire format for ILP packets).
 * Uses the real TOON codec from @crosstown/relay.
 */
function eventToBase64Toon(event: NostrEvent): string {
  const toonData = encodeEventToToon(event);
  return Buffer.from(toonData).toString('base64');
}

/**
 * Build a HandlePacketRequest matching the shape the SDK handler expects.
 */
function createPacketRequest(
  event: NostrEvent,
  amount: string | bigint,
  destination = 'g.agent.test'
) {
  return {
    amount: String(amount),
    destination,
    data: eventToBase64Toon(event),
  };
}

/**
 * Calculate the exact price for an event given a basePricePerByte.
 * Mirrors the pricing logic: toonBytes.length * basePricePerByte.
 */
function calculatePrice(event: NostrEvent, basePricePerByte: bigint): bigint {
  const toonData = encodeEventToToon(event);
  return BigInt(toonData.length) * basePricePerByte;
}

// ============================================================================
// Tests
// ============================================================================

/**
 * SKIP: @crosstown/sdk does not exist yet.
 *
 * These tests describe the EventStorageHandler that will be created as part
 * of the SDK. The handler receives incoming ILP packets, decodes the TOON
 * payload into a NostrEvent via ctx.decode(), validates payment, stores the
 * event, and returns ctx.accept() with metadata or ctx.reject() with an
 * ILP error code.
 */
describe.skip('EventStorageHandler', () => {
  const BASE_PRICE_PER_BYTE = 10n;

  let eventStore: SqliteEventStore;
  let nodeSk: Uint8Array;
  let nodePubkey: string;
  let handler: ReturnType<typeof createEventStorageHandler>;

  beforeEach(() => {
    // Real SQLite in-memory store -- no mocks
    eventStore = new SqliteEventStore(':memory:');

    // Node identity for self-write bypass
    nodeSk = generateSecretKey();
    nodePubkey = getPublicKey(nodeSk);

    // Create the SDK handler under test
    // WILL FAIL: createEventStorageHandler does not exist yet
    handler = createEventStorageHandler({
      eventStore,
      basePricePerByte: BASE_PRICE_PER_BYTE,
      ownerPubkey: nodePubkey,
      toonDecoder: decodeEventFromToon,
      toonEncoder: encodeEventToToon,
    });
  });

  afterEach(() => {
    eventStore.close();
  });

  // ---------------------------------------------------------------------------
  // P0: Core payment-gated storage
  // ---------------------------------------------------------------------------

  it('should store event when payment meets price', async () => {
    // FAILS because createEventStorageHandler is not implemented
    const event = createValidSignedEvent({ content: 'paid event' });
    const price = calculatePrice(event, BASE_PRICE_PER_BYTE);
    const request = createPacketRequest(event, price);

    const result = await handler(request);

    // Handler should accept the packet
    expect(result.accept).toBe(true);

    // Event should be persisted in the real SQLite store
    const stored = eventStore.get(event.id);
    expect(stored).toBeDefined();
    expect(stored?.id).toBe(event.id);
    expect(stored?.content).toBe('paid event');
  });

  it('should call ctx.decode() and get structured NostrEvent matching original', async () => {
    // FAILS because the SDK handler context API does not exist yet.
    // Verifies that ctx.decode() correctly round-trips through TOON encoding:
    // original event -> TOON bytes -> base64 -> handler -> ctx.decode() -> NostrEvent
    const event = createValidSignedEvent({
      kind: 1,
      content: 'decode roundtrip test',
      tags: [['p', 'deadbeef'.repeat(8)]],
    });
    const price = calculatePrice(event, BASE_PRICE_PER_BYTE);
    const request = createPacketRequest(event, price);

    const result = await handler(request);
    expect(result.accept).toBe(true);

    // The stored event must match every field of the original
    const stored = eventStore.get(event.id);
    expect(stored).toBeDefined();
    expect(stored!.id).toBe(event.id);
    expect(stored!.pubkey).toBe(event.pubkey);
    expect(stored!.kind).toBe(event.kind);
    expect(stored!.content).toBe(event.content);
    expect(stored!.tags).toEqual(event.tags);
    expect(stored!.created_at).toBe(event.created_at);
    expect(stored!.sig).toBe(event.sig);
  });

  it('should bypass pricing for node own pubkey (self-write)', async () => {
    // FAILS because the SDK handler self-write bypass is not implemented.
    // The node's own pubkey should be able to write events for free.
    const ownEvent = createValidSignedEvent(
      { content: 'self-write test' },
      nodeSk
    );

    // Send with amount=0 -- should succeed for owner
    const request = createPacketRequest(ownEvent, '0');
    const result = await handler(request);

    expect(result.accept).toBe(true);
    expect(result.metadata?.eventId).toBe(ownEvent.id);

    // Verify actually stored
    const stored = eventStore.get(ownEvent.id);
    expect(stored).toBeDefined();
    expect(stored?.pubkey).toBe(nodePubkey);
  });

  // ---------------------------------------------------------------------------
  // P1: TOON-native storage fidelity
  // ---------------------------------------------------------------------------

  it('should preserve TOON encoding in storage (roundtrip fidelity)', async () => {
    // FAILS because the SDK storage layer for TOON-native persistence is not built.
    // Events must be stored with original TOON encoding so that re-encoding
    // the retrieved event produces identical bytes (bit-for-bit roundtrip).
    const event = createValidSignedEvent({
      content: 'TOON fidelity test with unicode: \u00e9\u00e0\u00fc',
      tags: [
        ['t', 'test'],
        ['e', 'abc'.repeat(21)],
      ],
    });
    const originalToon = encodeEventToToon(event);
    const price = calculatePrice(event, BASE_PRICE_PER_BYTE);
    const request = createPacketRequest(event, price);

    await handler(request);

    // Retrieve the stored event and re-encode to TOON
    const stored = eventStore.get(event.id);
    expect(stored).toBeDefined();
    const reEncodedToon = encodeEventToToon(stored!);

    // TOON bytes must be identical (TOON-native: no lossy JSON intermediate)
    expect(Buffer.from(reEncodedToon).toString('base64')).toBe(
      Buffer.from(originalToon).toString('base64')
    );
  });

  it('should return eventId and storedAt in accept response', async () => {
    // FAILS because ctx.accept() metadata shape is not implemented.
    const event = createValidSignedEvent();
    const price = calculatePrice(event, BASE_PRICE_PER_BYTE);
    const request = createPacketRequest(event, price);

    const beforeMs = Date.now();
    const result = await handler(request);
    const afterMs = Date.now();

    expect(result.accept).toBe(true);
    if (!result.accept) throw new Error('unreachable');

    // eventId must match the Nostr event id
    expect(result.metadata?.eventId).toBe(event.id);

    // storedAt must be a reasonable timestamp (within the test window)
    expect(result.metadata?.storedAt).toBeTypeOf('number');
    expect(result.metadata!.storedAt).toBeGreaterThanOrEqual(beforeMs);
    expect(result.metadata!.storedAt).toBeLessThanOrEqual(afterMs);
  });

  // ---------------------------------------------------------------------------
  // P0: Payment rejection
  // ---------------------------------------------------------------------------

  it('should reject insufficient payment with F04 error code', async () => {
    // FAILS because the SDK handler error codes are not defined.
    // The ILP F04 code means "Insufficient Destination Amount".
    // (Note: existing BLS uses F06; the SDK normalizes to standard ILP F04.)
    const event = createValidSignedEvent({ content: 'underpaid' });
    const price = calculatePrice(event, BASE_PRICE_PER_BYTE);
    const insufficientAmount = price - 1n;

    const request = createPacketRequest(event, insufficientAmount);
    const result = await handler(request);

    expect(result.accept).toBe(false);
    if (result.accept) throw new Error('unreachable');

    // SDK uses standard ILP F04 (Insufficient Destination Amount)
    expect(result.code).toBe('F04');
    expect(result.message).toMatch(/insufficient/i);
    expect(result.metadata?.required).toBe(price.toString());
    expect(result.metadata?.received).toBe(insufficientAmount.toString());

    // Must NOT store the event
    const stored = eventStore.get(event.id);
    expect(stored).toBeUndefined();
  });

  // ---------------------------------------------------------------------------
  // P1: Signature validation
  // ---------------------------------------------------------------------------

  it('should reject invalid signature with F06 error code', async () => {
    // FAILS because the SDK handler signature validation is not implemented.
    // Tamper with a signed event to produce an invalid signature.
    const event = createValidSignedEvent({ content: 'original content' });
    const tamperedEvent = { ...event, content: 'tampered content' };

    // Encode the tampered event (valid TOON, invalid Nostr signature)
    const toonData = encodeEventToToon(tamperedEvent);
    const base64Data = Buffer.from(toonData).toString('base64');

    const request = {
      amount: '1000000', // More than enough
      destination: 'g.agent.test',
      data: base64Data,
    };

    const result = await handler(request);

    expect(result.accept).toBe(false);
    if (result.accept) throw new Error('unreachable');

    // SDK uses F06 for "Unexpected Payment" / bad data integrity
    expect(result.code).toBe('F06');
    expect(result.message).toMatch(/signature/i);

    // Must NOT store tampered event
    const stored = eventStore.get(event.id);
    expect(stored).toBeUndefined();
  });
});
