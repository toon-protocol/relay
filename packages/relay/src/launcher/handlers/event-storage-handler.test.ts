/**
 * Tests for Story 2.1: Relay Event Storage Handler
 *
 * GREEN PHASE: Tests updated from RED phase to match the real SDK architecture.
 * The handler receives HandlerContext (not raw HandlePacketRequest).
 * The SDK pipeline handles pricing, signature verification, and self-write bypass.
 * The handler's only job is: ctx.decode() -> store -> ctx.accept().
 *
 * Test approaches:
 *   Approach A (unit): Build HandlerContext via createTestContext() helper,
 *     call handler directly. Tests handler logic in isolation.
 *   Approach B (pipeline): Wire full createNode() with handler registered,
 *     send packets through SDK pipeline. Tests pipeline + handler integration.
 *     Required for pricing (F04) and signature (F06) since those happen in SDK.
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
  shallowParseToon,
} from '@toon-protocol/core/toon';
import { SqliteEventStore } from '@toon-protocol/relay';
import {
  createHandlerContext,
  createNode,
  type Handler,
  type HandlerContext,
} from '@toon-protocol/sdk';
import type {
  HandlePacketRequest,
  HandlePacketResponse,
  EmbeddableConnectorLike,
  SendPacketParams,
  SendPacketResult,
  RegisterPeerParams,
} from '@toon-protocol/core';

// Import from the local module under test (Town package)
import { createEventStorageHandler } from './event-storage-handler.js';

// ============================================================================
// Test Factories
// ============================================================================

/** Deterministic timestamp for reproducible tests (2026-01-01T00:00:00Z) */
const TEST_CREATED_AT = 1767225600;

/**
 * Create a properly signed Nostr event for testing.
 * Uses real nostr-tools signing (not mocked).
 */
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
 * Uses the real TOON codec from @toon-protocol/core/toon.
 */
function eventToBase64Toon(event: NostrEvent): string {
  const toonData = encodeEventToToon(event);
  return Buffer.from(toonData).toString('base64');
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
// Test Helper: Build HandlerContext from request data (Approach A)
// ============================================================================

/**
 * Build a HandlerContext suitable for calling the handler directly.
 * This bypasses the SDK pipeline (no pricing, no signature verification).
 * Used for unit-level tests (Approach A) that test handler logic in isolation.
 */
function createTestContext(request: {
  amount: string | bigint;
  destination: string;
  data: string;
}): HandlerContext {
  const toonBytes = Buffer.from(request.data, 'base64');
  const meta = shallowParseToon(toonBytes);
  return createHandlerContext({
    toon: request.data,
    meta,
    amount: BigInt(request.amount),
    destination: request.destination,
    toonDecoder: (toon: string) => {
      const bytes = Buffer.from(toon, 'base64');
      return decodeEventFromToon(bytes);
    },
  });
}

// ============================================================================
// Test Helper: Mock connector for pipeline tests (Approach B)
// ============================================================================

/**
 * Create a minimal mock connector for pipeline integration tests.
 * Captures the packetHandler registered by createNode() so tests can
 * invoke it directly with HandlePacketRequest payloads.
 */
function createMockConnector(): EmbeddableConnectorLike & {
  packetHandler:
    | ((
        req: HandlePacketRequest
      ) => HandlePacketResponse | Promise<HandlePacketResponse>)
    | null;
} {
  return {
    packetHandler: null,
    async sendPacket(_params: SendPacketParams): Promise<SendPacketResult> {
      return { type: 'reject', code: 'F02', message: 'No route' };
    },
    async registerPeer(_params: RegisterPeerParams): Promise<void> {},
    async removePeer(_peerId: string): Promise<void> {},
    setPacketHandler(
      handler: (
        req: HandlePacketRequest
      ) => HandlePacketResponse | Promise<HandlePacketResponse>
    ): void {
      this.packetHandler = handler;
    },
  };
}

// ============================================================================
// Unit Tests (Approach A) - Handler logic in isolation
// ============================================================================

describe('EventStorageHandler', () => {
  let eventStore: SqliteEventStore;
  let handler: Handler;

  beforeEach(() => {
    // Real SQLite in-memory store -- no mocks
    eventStore = new SqliteEventStore(':memory:');

    // Create the handler under test with simplified config (eventStore only)
    // The SDK pipeline handles pricing, verification, and self-write bypass.
    handler = createEventStorageHandler({ eventStore });
  });

  afterEach(() => {
    eventStore.close();
  });

  // ---------------------------------------------------------------------------
  // T-2.1-01 [P0]: Core payment-gated storage (AC #1)
  // ---------------------------------------------------------------------------

  it('should store event when payment meets price', async () => {
    // Given: a valid signed event with sufficient payment
    const event = createValidSignedEvent({ content: 'paid event' });
    const ctx = createTestContext({
      amount: '1000000',
      destination: 'g.agent.test',
      data: eventToBase64Toon(event),
    });

    // When: the handler processes the context
    const result = await handler(ctx);

    // Then: handler should accept the packet
    expect(result.accept).toBe(true);

    // And: event should be persisted in the real SQLite store
    const stored = eventStore.get(event.id);
    expect(stored).toBeDefined();
    expect(stored?.id).toBe(event.id);
    expect(stored?.content).toBe('paid event');
  });

  // ---------------------------------------------------------------------------
  // T-2.1-02 [P0]: ctx.decode() roundtrip fidelity (AC #1, #2)
  // ---------------------------------------------------------------------------

  it('should call ctx.decode() and get structured NostrEvent matching original', async () => {
    // Given: a valid event with tags and specific content
    // Verifies that ctx.decode() correctly round-trips through TOON encoding:
    // original event -> TOON bytes -> base64 -> handler -> ctx.decode() -> NostrEvent
    const event = createValidSignedEvent({
      kind: 1,
      content: 'decode roundtrip test',
      tags: [['p', 'deadbeef'.repeat(8)]],
    });
    const ctx = createTestContext({
      amount: '1000000',
      destination: 'g.agent.test',
      data: eventToBase64Toon(event),
    });

    // When: the handler processes the context
    const result = await handler(ctx);

    // Then: handler should accept
    expect(result.accept).toBe(true);

    // And: the stored event must match every field of the original
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

  // ---------------------------------------------------------------------------
  // T-2.1-04 [P1]: TOON-native storage fidelity (AC #2)
  // ---------------------------------------------------------------------------

  it('should preserve TOON encoding in storage (roundtrip fidelity)', async () => {
    // Given: an event with unicode content and multiple tags
    // Events must be stored such that re-encoding produces identical TOON bytes.
    const event = createValidSignedEvent({
      content: 'TOON fidelity test with unicode: \u00e9\u00e0\u00fc',
      tags: [
        ['t', 'test'],
        ['e', 'abc'.repeat(21)],
      ],
    });
    const ctx = createTestContext({
      amount: '1000000',
      destination: 'g.agent.test',
      data: eventToBase64Toon(event),
    });

    // When: the handler stores the event
    await handler(ctx);

    // Then: retrieving and re-encoding produces a decodable TOON that matches
    // the original event semantically (all field values identical).
    // Note: TOON field ordering depends on JS property iteration order, which
    // differs between nostr-tools finalizeEvent() output and SqliteEventStore
    // get() output. Semantic equality is what matters for roundtrip fidelity.
    const stored = eventStore.get(event.id);
    expect(stored).toBeDefined();
    const reEncodedToon = encodeEventToToon(stored!);

    // Decode both TOON representations and verify semantic equality
    const originalDecoded = decodeEventFromToon(encodeEventToToon(event));
    const reDecoded = decodeEventFromToon(reEncodedToon);
    expect(reDecoded.id).toBe(originalDecoded.id);
    expect(reDecoded.pubkey).toBe(originalDecoded.pubkey);
    expect(reDecoded.kind).toBe(originalDecoded.kind);
    expect(reDecoded.content).toBe(originalDecoded.content);
    expect(reDecoded.tags).toEqual(originalDecoded.tags);
    expect(reDecoded.created_at).toBe(originalDecoded.created_at);
    expect(reDecoded.sig).toBe(originalDecoded.sig);
  });

  // ---------------------------------------------------------------------------
  // T-2.1-05 [P1]: Accept response metadata (AC #1)
  // ---------------------------------------------------------------------------

  it('should return eventId and storedAt in accept response', async () => {
    // Given: a valid event
    const event = createValidSignedEvent();
    const ctx = createTestContext({
      amount: '1000000',
      destination: 'g.agent.test',
      data: eventToBase64Toon(event),
    });

    // When: the handler processes the context
    const beforeMs = Date.now();
    const result = await handler(ctx);
    const afterMs = Date.now();

    // Then: handler accepts with metadata
    expect(result.accept).toBe(true);
    if (!result.accept) throw new Error('unreachable');

    // And: eventId must match the Nostr event id
    expect(result.metadata?.['eventId']).toBe(event.id);

    // And: storedAt must be a reasonable timestamp (within the test window)
    expect(result.metadata?.['storedAt']).toBeTypeOf('number');
    expect(result.metadata!['storedAt']).toBeGreaterThanOrEqual(beforeMs);
    expect(result.metadata!['storedAt']).toBeLessThanOrEqual(afterMs);
  });

  // ---------------------------------------------------------------------------
  // T-2.1-09 [P1]: General event kinds -- replaceable event (AC #1)
  // ---------------------------------------------------------------------------

  it('should store replaceable event kind (general event kinds)', async () => {
    // Given: a replaceable event (kind 10002 - relay list, in NIP-65 range 10000-19999)
    // AC #1 says "handler registered for general event kinds" -- verify the handler
    // works with non-kind-1 events, specifically replaceable event kinds.
    const event = createValidSignedEvent({
      kind: 10002,
      content: '',
      tags: [
        ['r', 'wss://relay.example.com'],
        ['r', 'wss://relay2.example.com', 'read'],
      ],
    });
    const ctx = createTestContext({
      amount: '1000000',
      destination: 'g.agent.test',
      data: eventToBase64Toon(event),
    });

    // When: the handler processes the replaceable event
    const result = await handler(ctx);

    // Then: handler should accept the packet
    expect(result.accept).toBe(true);

    // And: the replaceable event should be stored with correct kind
    const stored = eventStore.get(event.id);
    expect(stored).toBeDefined();
    expect(stored?.kind).toBe(10002);
    expect(stored?.tags).toEqual(event.tags);
  });

  // ---------------------------------------------------------------------------
  // T-2.1-10 [P1]: Duplicate event idempotency (AC #1, #2)
  // ---------------------------------------------------------------------------

  it('should accept duplicate event without error (idempotent storage)', async () => {
    // Given: a valid event that has already been stored once
    // The EventStore uses INSERT OR IGNORE for duplicates, so storing the same
    // event twice should not throw. The handler should accept both times.
    const event = createValidSignedEvent({ content: 'duplicate test' });
    const base64Toon = eventToBase64Toon(event);

    const ctx1 = createTestContext({
      amount: '1000000',
      destination: 'g.agent.test',
      data: base64Toon,
    });

    // When: the handler stores the event the first time
    const result1 = await handler(ctx1);
    expect(result1.accept).toBe(true);

    // And: the handler is called again with the same event
    const ctx2 = createTestContext({
      amount: '1000000',
      destination: 'g.agent.test',
      data: base64Toon,
    });
    const result2 = await handler(ctx2);

    // Then: both calls should accept (no error on duplicate)
    expect(result2.accept).toBe(true);

    // And: the stored event should still be retrievable and correct
    const stored = eventStore.get(event.id);
    expect(stored).toBeDefined();
    expect(stored?.id).toBe(event.id);
    expect(stored?.content).toBe('duplicate test');
  });

  // ---------------------------------------------------------------------------
  // T-2.1-08 [P1]: ctx.toon raw TOON passthrough (AC #1)
  // ---------------------------------------------------------------------------

  it('should receive ctx.toon as raw TOON string (no premature decode)', async () => {
    // Given: a valid event encoded to base64 TOON
    const event = createValidSignedEvent({ content: 'toon passthrough' });
    const base64Toon = eventToBase64Toon(event);
    const ctx = createTestContext({
      amount: '1000000',
      destination: 'g.agent.test',
      data: base64Toon,
    });

    // Then: ctx.toon should be the raw base64 TOON string (before any decode)
    expect(ctx.toon).toBe(base64Toon);
    expect(typeof ctx.toon).toBe('string');

    // And: calling the handler should still work (decode happens inside handler)
    const result = await handler(ctx);
    expect(result.accept).toBe(true);

    // And: ctx.toon should still be the original base64 string after handler runs
    expect(ctx.toon).toBe(base64Toon);
  });
});

// ============================================================================
// Pipeline Integration Tests (Approach B) - Full SDK pipeline
// ============================================================================

describe('EventStorageHandler (pipeline integration)', () => {
  const BASE_PRICE_PER_BYTE = 10n;

  let eventStore: SqliteEventStore;
  let nodeSk: Uint8Array;
  let nodePubkey: string;
  let activeNode: { stop(): Promise<void> } | null = null;

  beforeEach(() => {
    eventStore = new SqliteEventStore(':memory:');
    nodeSk = generateSecretKey();
    nodePubkey = getPublicKey(nodeSk);
    activeNode = null;
  });

  afterEach(async () => {
    // Ensure node is stopped even if a test assertion fails mid-test
    if (activeNode) {
      await activeNode.stop();
      activeNode = null;
    }
    eventStore.close();
  });

  // ---------------------------------------------------------------------------
  // T-2.1-11 [P0]: Successful exact payment through full pipeline (AC #1)
  // ---------------------------------------------------------------------------

  it('should accept exact payment from external pubkey through pipeline', async () => {
    // Given: a node with the event storage handler and pricing enabled
    // AC #1 says "when a paid ILP packet arrives" through a createNode() instance.
    // This test verifies the full happy path through the pipeline: paid packet from
    // an external pubkey -> shallow parse -> Schnorr verify -> pricing accept ->
    // handler -> store -> ctx.accept().
    const mockConnector = createMockConnector();
    const handler = createEventStorageHandler({ eventStore });

    const node = createNode({
      secretKey: nodeSk,
      connector: mockConnector,
      defaultHandler: handler,
      basePricePerByte: BASE_PRICE_PER_BYTE,
      knownPeers: [],
    });
    activeNode = node;

    // start() wires the packet handler via setPacketHandler
    await node.start();
    expect(mockConnector.packetHandler).not.toBeNull();

    // And: an event from a DIFFERENT key (not self-write), with exact payment
    const otherSk = generateSecretKey();
    const event = createValidSignedEvent(
      { content: 'paid through pipeline' },
      otherSk
    );
    const exactPrice = calculatePrice(event, BASE_PRICE_PER_BYTE);

    // When: the packet is sent with exact payment through the pipeline
    const request: HandlePacketRequest = {
      amount: exactPrice.toString(),
      destination: 'g.agent.test',
      data: eventToBase64Toon(event),
    };
    const result = await mockConnector.packetHandler!(request);

    // Then: SDK pipeline accepts (pricing passes, signature valid)
    expect(result.accept).toBe(true);
    if (!result.accept) throw new Error('unreachable');
    expect(result.metadata?.['eventId']).toBe(event.id);

    // And: the event should be stored in SQLite
    const stored = eventStore.get(event.id);
    expect(stored).toBeDefined();
    expect(stored?.id).toBe(event.id);
    expect(stored?.content).toBe('paid through pipeline');
    expect(stored?.pubkey).toBe(getPublicKey(otherSk));
  });

  // ---------------------------------------------------------------------------
  // T-2.1-03 [P0]: Self-write bypass through SDK pipeline (AC #3)
  // ---------------------------------------------------------------------------

  it('should bypass pricing for node own pubkey (self-write)', async () => {
    // Given: a node with secretKey and the event storage handler as default handler
    const mockConnector = createMockConnector();
    const handler = createEventStorageHandler({ eventStore });

    const node = createNode({
      secretKey: nodeSk,
      connector: mockConnector,
      defaultHandler: handler,
      basePricePerByte: BASE_PRICE_PER_BYTE,
      knownPeers: [],
    });
    activeNode = node;

    // start() wires the packet handler via setPacketHandler
    await node.start();
    expect(mockConnector.packetHandler).not.toBeNull();

    // And: an event signed by the node's own key
    const ownEvent = createValidSignedEvent(
      { content: 'self-write test' },
      nodeSk
    );

    // Verify the event's pubkey matches the node's pubkey
    expect(ownEvent.pubkey).toBe(nodePubkey);

    // When: the packet is sent with amount=0 through the pipeline
    const request: HandlePacketRequest = {
      amount: '0',
      destination: 'g.agent.test',
      data: eventToBase64Toon(ownEvent),
    };
    const result = await mockConnector.packetHandler!(request);

    // Then: handler should accept (self-write bypasses pricing)
    expect(result.accept).toBe(true);
    if (!result.accept) throw new Error('unreachable');
    expect(result.metadata?.['eventId']).toBe(ownEvent.id);

    // And: the event should be stored
    const stored = eventStore.get(ownEvent.id);
    expect(stored).toBeDefined();
    expect(stored?.pubkey).toBe(nodePubkey);
  });

  // ---------------------------------------------------------------------------
  // T-2.1-06 [P0]: Pricing rejection through SDK pipeline (AC #1)
  // ---------------------------------------------------------------------------

  it('should reject insufficient payment with F04 error code', async () => {
    // Given: a node with the event storage handler and pricing enabled
    const mockConnector = createMockConnector();
    const handler = createEventStorageHandler({ eventStore });

    const node = createNode({
      secretKey: nodeSk,
      connector: mockConnector,
      defaultHandler: handler,
      basePricePerByte: BASE_PRICE_PER_BYTE,
      knownPeers: [],
    });
    activeNode = node;

    // start() wires the packet handler via setPacketHandler
    await node.start();
    expect(mockConnector.packetHandler).not.toBeNull();

    // And: an event from a DIFFERENT key (not self-write)
    const otherSk = generateSecretKey();
    const event = createValidSignedEvent({ content: 'underpaid' }, otherSk);
    const price = calculatePrice(event, BASE_PRICE_PER_BYTE);
    const insufficientAmount = price - 1n;

    // When: the packet is sent with insufficient payment
    const request: HandlePacketRequest = {
      amount: insufficientAmount.toString(),
      destination: 'g.agent.test',
      data: eventToBase64Toon(event),
    };
    const result = await mockConnector.packetHandler!(request);

    // Then: SDK pipeline rejects with F04 (Insufficient Destination Amount)
    expect(result.accept).toBe(false);
    if (result.accept) throw new Error('unreachable');
    expect(result.code).toBe('F04');
    expect(result.message).toMatch(/insufficient/i);

    // And: event must NOT be stored
    const stored = eventStore.get(event.id);
    expect(stored).toBeUndefined();
  });

  // ---------------------------------------------------------------------------
  // T-2.1-07 [P1]: Signature rejection through SDK pipeline
  // ---------------------------------------------------------------------------

  it('should reject invalid signature with F06 error code', async () => {
    // Given: a node with the event storage handler and verification enabled
    const mockConnector = createMockConnector();
    const handler = createEventStorageHandler({ eventStore });

    const node = createNode({
      secretKey: nodeSk,
      connector: mockConnector,
      defaultHandler: handler,
      basePricePerByte: BASE_PRICE_PER_BYTE,
      knownPeers: [],
    });
    activeNode = node;

    // start() wires the packet handler via setPacketHandler
    await node.start();
    expect(mockConnector.packetHandler).not.toBeNull();

    // And: a tampered event (valid TOON structure, forged Schnorr signature)
    // Create a valid event, then replace the sig with one from a different event.
    // The TOON structure is valid, the sig is valid hex, but it does not match
    // the id -- so Schnorr verification fails.
    const event = createValidSignedEvent({ content: 'original content' });
    const otherEvent = createValidSignedEvent({ content: 'other content' });
    // Replace the sig with a different event's sig (valid hex, wrong value)
    const forgedEvent = { ...event, sig: otherEvent.sig };
    const toonData = encodeEventToToon(forgedEvent);
    const base64Data = Buffer.from(toonData).toString('base64');

    // When: the tampered packet is sent through the pipeline
    const request: HandlePacketRequest = {
      amount: '1000000',
      destination: 'g.agent.test',
      data: base64Data,
    };
    const result = await mockConnector.packetHandler!(request);

    // Then: SDK pipeline rejects with F06 (signature verification failed)
    expect(result.accept).toBe(false);
    if (result.accept) throw new Error('unreachable');
    expect(result.code).toBe('F06');
    expect(result.message).toMatch(/signature/i);

    // And: tampered event must NOT be stored
    const stored = eventStore.get(event.id);
    expect(stored).toBeUndefined();
  });
});

// ============================================================================
// DVM Event Storage and Provider Subscription Tests (Story 5.2, ACs 1 + 3)
// ============================================================================

describe('EventStorageHandler DVM Events (Story 5.2)', () => {
  let eventStore: SqliteEventStore;
  let handler: Handler;

  beforeEach(() => {
    eventStore = new SqliteEventStore(':memory:');
    handler = createEventStorageHandler({ eventStore });
  });

  afterEach(() => {
    eventStore.close();
  });

  // AC 1: relay stores the DVM event
  it('[P0] AC-1: Kind 5100 DVM job request event is stored by createEventStorageHandler()', async () => {
    // Arrange -- create a Kind 5100 DVM event with NIP-90 tags
    const sk = generateSecretKey();
    const dvmEvent = createValidSignedEvent(
      {
        kind: 5100,
        content: 'Summarize this article',
        tags: [
          ['i', 'Quantum computing breakthrough in 2026', 'text'],
          ['bid', '5000000', 'usdc'],
          ['output', 'text/plain'],
          ['p', 'de'.repeat(32)],
          ['param', 'model', 'claude-3'],
          ['param', 'max_tokens', '1000'],
          ['relays', 'wss://relay1.example.com', 'wss://relay2.example.com'],
        ],
      },
      sk
    );

    const ctx = createTestContext({
      amount: '1000000',
      destination: 'g.toon.relay',
      data: eventToBase64Toon(dvmEvent),
    });

    // Act
    const result = await handler(ctx);

    // Assert -- handler accepts
    expect(result.accept).toBe(true);

    // Assert -- DVM event is stored
    const stored = eventStore.get(dvmEvent.id);
    expect(stored).toBeDefined();
    expect(stored?.kind).toBe(5100);
    expect(stored?.content).toBe('Summarize this article');

    // Assert -- all DVM tags survive storage
    const iTag = stored?.tags.find((t: string[]) => t[0] === 'i');
    expect(iTag).toBeDefined();
    expect(iTag?.[1]).toBe('Quantum computing breakthrough in 2026');
    expect(iTag?.[2]).toBe('text');

    const bidTag = stored?.tags.find((t: string[]) => t[0] === 'bid');
    expect(bidTag).toBeDefined();
    expect(bidTag?.[1]).toBe('5000000');
    expect(bidTag?.[2]).toBe('usdc');

    const outputTag = stored?.tags.find((t: string[]) => t[0] === 'output');
    expect(outputTag?.[1]).toBe('text/plain');

    const pTag = stored?.tags.find((t: string[]) => t[0] === 'p');
    expect(pTag?.[1]).toBe('de'.repeat(32));

    const paramTags = stored?.tags.filter((t: string[]) => t[0] === 'param');
    expect(paramTags).toHaveLength(2);

    const relaysTag = stored?.tags.find((t: string[]) => t[0] === 'relays');
    expect(relaysTag).toBeDefined();
    expect(relaysTag?.[1]).toBe('wss://relay1.example.com');
  });

  // AC 3 / T-5.2-07: provider can query stored DVM events by kind filter
  it('[P1] T-5.2-07: provider can query stored DVM events by kind filter (subscription backing)', async () => {
    // Arrange -- store a mix of regular and DVM events
    const sk = generateSecretKey();

    const regularEvent = createValidSignedEvent(
      { kind: 1, content: 'Regular note' },
      sk
    );
    const dvmEvent5100 = createValidSignedEvent(
      {
        kind: 5100,
        content: 'Text gen request',
        tags: [
          ['i', 'Test input', 'text'],
          ['bid', '1000', 'usdc'],
          ['output', 'text/plain'],
        ],
      },
      sk
    );
    const dvmEvent5200 = createValidSignedEvent(
      {
        kind: 5200,
        content: 'Image gen request',
        tags: [
          ['i', 'Generate a cat', 'text'],
          ['bid', '2000', 'usdc'],
          ['output', 'image/png'],
        ],
      },
      sk
    );

    // Store all events via handler
    for (const event of [regularEvent, dvmEvent5100, dvmEvent5200]) {
      const ctx = createTestContext({
        amount: '1000000',
        destination: 'g.toon.relay',
        data: eventToBase64Toon(event),
      });
      const result = await handler(ctx);
      expect(result.accept).toBe(true);
    }

    // Act -- provider queries for Kind 5100 only (filter by kind)
    const kind5100Results = eventStore.query([{ kinds: [5100] }]);

    // Assert -- only Kind 5100 events returned
    expect(kind5100Results).toHaveLength(1);
    expect(kind5100Results[0]?.kind).toBe(5100);
    expect(kind5100Results[0]?.id).toBe(dvmEvent5100.id);

    // Act -- provider queries for all DVM request kinds (5100, 5200)
    const allDvmResults = eventStore.query([{ kinds: [5100, 5200] }]);

    // Assert -- both DVM events returned, not the regular event
    expect(allDvmResults).toHaveLength(2);
    const returnedKinds = allDvmResults.map((e) => e.kind);
    expect(returnedKinds).toContain(5100);
    expect(returnedKinds).toContain(5200);

    // Act -- provider queries for Kind 1 (regular notes only)
    const regularResults = eventStore.query([{ kinds: [1] }]);

    // Assert -- only regular events returned
    expect(regularResults).toHaveLength(1);
    expect(regularResults[0]?.kind).toBe(1);
  });

  // AC 3: DVM events stored via pipeline are queryable by kind
  it('[P1] AC-3: DVM event stored via pipeline is queryable by kind (provider subscription simulation)', async () => {
    // Arrange -- use pipeline approach (Approach B) for full fidelity
    const nodeSk = generateSecretKey();
    const mockConnector = createMockConnector();

    const node = createNode({
      secretKey: nodeSk,
      connector: mockConnector,
      defaultHandler: handler,
      basePricePerByte: 10n,
      knownPeers: [],
    });

    await node.start();
    expect(mockConnector.packetHandler).not.toBeNull();

    // Create a DVM event signed by a different key
    const customerSk = generateSecretKey();
    const dvmEvent = createValidSignedEvent(
      {
        kind: 5100,
        content: 'Pipeline stored DVM event',
        tags: [
          ['i', 'Test pipeline storage', 'text'],
          ['bid', '5000000', 'usdc'],
          ['output', 'text/plain'],
        ],
      },
      customerSk
    );
    const exactPrice = calculatePrice(dvmEvent, 10n);

    // Act -- send through pipeline
    const result = await mockConnector.packetHandler!({
      amount: exactPrice.toString(),
      destination: 'g.toon.relay',
      data: eventToBase64Toon(dvmEvent),
    });

    // Assert -- pipeline accepts
    expect(result.accept).toBe(true);

    // Assert -- provider can query the stored DVM event by kind
    const queryResults = eventStore.query([{ kinds: [5100] }]);
    expect(queryResults).toHaveLength(1);
    expect(queryResults[0]?.id).toBe(dvmEvent.id);
    expect(queryResults[0]?.kind).toBe(5100);
    expect(queryResults[0]?.content).toBe('Pipeline stored DVM event');

    // Assert -- DVM tags survived pipeline + storage
    const iTag = queryResults[0]?.tags.find((t: string[]) => t[0] === 'i');
    expect(iTag?.[1]).toBe('Test pipeline storage');
    expect(iTag?.[2]).toBe('text');

    const bidTag = queryResults[0]?.tags.find((t: string[]) => t[0] === 'bid');
    expect(bidTag?.[1]).toBe('5000000');
    expect(bidTag?.[2]).toBe('usdc');

    // Cleanup
    await node.stop();
  });
});
