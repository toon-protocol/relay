/**
 * Tests for Story 2.2: SPSP Handshake Handler
 *
 * GREEN PHASE: All tests pass. The handler lives in @crosstown/town
 * (not @crosstown/sdk) and reimplements the SPSP request/response flow
 * as a kind-based handler (kind:23194), replacing the inline SPSP logic
 * in docker/src/entrypoint.ts.
 *
 * Infrastructure: Real NIP-44 encryption, real nostr-tools key generation,
 * real TOON codec. Connector transport is mocked.
 *
 * Test approach: Approach A (unit) -- build HandlerContext via
 * createTestContext() helper, call handler directly. Tests handler logic
 * in isolation. Pipeline integration tested in Story 2.3.
 */

import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
  generateSecretKey,
  getPublicKey,
  finalizeEvent,
} from 'nostr-tools/pure';
import { nip44 } from 'nostr-tools';
import type { NostrEvent } from 'nostr-tools/pure';
import {
  encodeEventToToon,
  decodeEventFromToon,
  shallowParseToon,
} from '@crosstown/core/toon';
import { SqliteEventStore } from '@crosstown/relay';
import {
  buildSpspRequestEvent,
  SPSP_RESPONSE_KIND,
  type SpspResponse,
  type SettlementNegotiationConfig,
  type ConnectorChannelClient,
  type ConnectorAdminClient,
} from '@crosstown/core';
import {
  createHandlerContext,
  type Handler,
  type HandlerContext,
} from '@crosstown/sdk';

// Import from the local module under test (Town package)
import { createSpspHandshakeHandler } from './spsp-handshake-handler.js';

// ============================================================================
// Test Factories
// ============================================================================

/**
 * Create a Nostr keypair for testing.
 */
function createKeypair() {
  const sk = generateSecretKey();
  const pubkey = getPublicKey(sk);
  return { sk, pubkey };
}

/**
 * Build a signed, encrypted kind:23194 SPSP request event using the real
 * @crosstown/core builder. Returns the event and requestId for correlation.
 */
function createSpspRequest(
  senderSk: Uint8Array,
  recipientPubkey: string,
  settlementInfo?: {
    ilpAddress?: string;
    supportedChains?: string[];
    settlementAddresses?: Record<string, string>;
    preferredTokens?: Record<string, string>;
  }
) {
  return buildSpspRequestEvent(recipientPubkey, senderSk, settlementInfo);
}

/**
 * Create a mock ConnectorChannelClient for settlement tests.
 */
function createMockChannelClient(): ConnectorChannelClient {
  return {
    openChannel: vi.fn().mockResolvedValue({
      channelId: `channel-${crypto.randomUUID().slice(0, 8)}`,
      status: 'open',
    }),
    getChannelState: vi.fn().mockResolvedValue({
      channelId: 'channel-mock',
      status: 'open' as const,
      chain: 'evm:base:31337',
    }),
  };
}

/**
 * Create a mock ConnectorAdminClient for peer registration tests.
 */
function createMockAdminClient(): ConnectorAdminClient {
  return {
    addPeer: vi.fn().mockResolvedValue(undefined),
    removePeer: vi.fn().mockResolvedValue(undefined),
  } as unknown as ConnectorAdminClient;
}

/**
 * Build a HandlePacketRequest from a signed SPSP request event.
 */
function createPacketRequest(event: NostrEvent, amount = '0') {
  const toonData = encodeEventToToon(event);
  return {
    amount,
    destination: 'g.agent.test',
    data: Buffer.from(toonData).toString('base64'),
  };
}

/**
 * Decrypt and parse an SPSP response event's content using NIP-44.
 */
function decryptSpspResponse(
  responseEvent: NostrEvent,
  recipientSk: Uint8Array,
  senderPubkey: string
): SpspResponse {
  const conversationKey = nip44.getConversationKey(recipientSk, senderPubkey);
  const decrypted = nip44.decrypt(responseEvent.content, conversationKey);
  return JSON.parse(decrypted) as SpspResponse;
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
  amount: string;
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
// Tests
// ============================================================================

/**
 * SpspHandshakeHandler tests.
 *
 * The handler intercepts kind:23194 events from incoming ILP packets,
 * generates fresh SPSP parameters, optionally negotiates settlement
 * chains, opens payment channels, and returns an encrypted kind:23195 response.
 *
 * The handler returns { accept: true, fulfillment: 'default-fulfillment', data }
 * directly (bypasses ctx.accept()) because the TOON-encoded SPSP response must
 * be in the top-level `data` field for the connector to relay it back.
 */
describe('SpspHandshakeHandler', () => {
  // Node identity (the "responder" in SPSP handshake)
  let nodeSk: Uint8Array;
  let nodePubkey: string;
  const nodeIlpAddress = 'g.test.node';

  // Requester identity (the "sender" initiating the SPSP handshake)
  let requesterSk: Uint8Array;
  let requesterPubkey: string;

  let eventStore: SqliteEventStore;
  let mockChannelClient: ReturnType<typeof createMockChannelClient>;
  let mockAdminClient: ReturnType<typeof createMockAdminClient>;
  let handler: Handler;

  // Settlement config for this node
  const settlementConfig: SettlementNegotiationConfig = {
    ownSupportedChains: ['evm:base:31337'],
    ownSettlementAddresses: {
      'evm:base:31337': '0xNodeAddress1234567890abcdef1234567890abcdef',
    },
    ownPreferredTokens: {
      'evm:base:31337': '0x5FbDB2315678afecb367f032d93F642f64180aa3',
    },
    ownTokenNetworks: {
      'evm:base:31337': '0xCafac3dD18aC6c6e92c921884f9E4176737C052c',
    },
    initialDeposit: '0',
    settlementTimeout: 86400,
  };

  beforeEach(() => {
    // Fresh identities for each test
    const node = createKeypair();
    nodeSk = node.sk;
    nodePubkey = node.pubkey;

    const requester = createKeypair();
    requesterSk = requester.sk;
    requesterPubkey = requester.pubkey;

    // Real SQLite in-memory store
    eventStore = new SqliteEventStore(':memory:');

    mockChannelClient = createMockChannelClient();
    mockAdminClient = createMockAdminClient();

    // Create the handler under test (lives in @crosstown/town, not SDK)
    handler = createSpspHandshakeHandler({
      secretKey: nodeSk,
      ilpAddress: nodeIlpAddress,
      eventStore,
      settlementConfig,
      channelClient: mockChannelClient,
      adminClient: mockAdminClient,
    });
  });

  afterEach(() => {
    eventStore.close();
  });

  // ---------------------------------------------------------------------------
  // P0: Core SPSP request/response flow
  // ---------------------------------------------------------------------------

  it('should process kind:23194 SPSP request and return encrypted response', async () => {
    // Builds a real NIP-44 encrypted SPSP request and verifies the handler
    // returns an accept response with encrypted SPSP response data.
    const { event, requestId } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
    });
    const request = createPacketRequest(event);

    const result = await handler(createTestContext(request));

    expect(result.accept).toBe(true);
    if (!result.accept) throw new Error('unreachable');

    // The response should include TOON-encoded SPSP response event data
    expect(result.data).toBeDefined();
    expect(typeof result.data).toBe('string');

    // Decode the response event from TOON
    const responseToonBytes = Uint8Array.from(
      Buffer.from(result.data!, 'base64')
    );
    const responseEvent = decodeEventFromToon(responseToonBytes);

    // Response should be kind:23195 from the node
    expect(responseEvent.kind).toBe(SPSP_RESPONSE_KIND);
    expect(responseEvent.pubkey).toBe(nodePubkey);

    // Decrypt and verify SPSP response payload
    const spspResponse = decryptSpspResponse(
      responseEvent,
      requesterSk,
      nodePubkey
    );
    expect(spspResponse.requestId).toBe(requestId);
    expect(spspResponse.destinationAccount).toMatch(/^g\.test\.node\.spsp\./);
    expect(spspResponse.sharedSecret).toBeDefined();
    expect(typeof spspResponse.sharedSecret).toBe('string');
    // Shared secret should be base64 encoding of 32 bytes
    expect(Buffer.from(spspResponse.sharedSecret, 'base64')).toHaveLength(32);
  });

  it('should generate unique SPSP parameters per request', async () => {
    // Each SPSP request must receive a unique destinationAccount and sharedSecret.
    const { event: event1 } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
    });
    const { event: event2 } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
    });

    const result1 = await handler(
      createTestContext(createPacketRequest(event1))
    );
    const result2 = await handler(
      createTestContext(createPacketRequest(event2))
    );

    expect(result1.accept).toBe(true);
    expect(result2.accept).toBe(true);
    if (!result1.accept || !result2.accept) throw new Error('unreachable');

    // Decode both responses
    const resp1Event = decodeEventFromToon(
      Uint8Array.from(Buffer.from(result1.data!, 'base64'))
    );
    const resp2Event = decodeEventFromToon(
      Uint8Array.from(Buffer.from(result2.data!, 'base64'))
    );

    const spsp1 = decryptSpspResponse(resp1Event, requesterSk, nodePubkey);
    const spsp2 = decryptSpspResponse(resp2Event, requesterSk, nodePubkey);

    // Destination accounts must be unique
    expect(spsp1.destinationAccount).not.toBe(spsp2.destinationAccount);

    // Shared secrets must be unique
    expect(spsp1.sharedSecret).not.toBe(spsp2.sharedSecret);
  });

  // ---------------------------------------------------------------------------
  // P0: Settlement negotiation
  // ---------------------------------------------------------------------------

  it('should negotiate settlement chain when both parties have overlapping chains', async () => {
    // Both parties support evm:base:31337, so negotiation should succeed.
    const { event } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
      supportedChains: ['evm:base:31337'],
      settlementAddresses: {
        'evm:base:31337': '0xRequesterAddr1234567890abcdef1234567890abcd',
      },
    });
    const request = createPacketRequest(event);

    const result = await handler(createTestContext(request));
    expect(result.accept).toBe(true);
    if (!result.accept) throw new Error('unreachable');

    // Decode and decrypt response
    const responseEvent = decodeEventFromToon(
      Uint8Array.from(Buffer.from(result.data!, 'base64'))
    );
    const spspResponse = decryptSpspResponse(
      responseEvent,
      requesterSk,
      nodePubkey
    );

    // Settlement fields should be populated
    expect(spspResponse.negotiatedChain).toBe('evm:base:31337');
    expect(spspResponse.settlementAddress).toBe(
      settlementConfig.ownSettlementAddresses['evm:base:31337']
    );
    expect(spspResponse.tokenAddress).toBe(
      settlementConfig.ownPreferredTokens!['evm:base:31337']
    );
    expect(spspResponse.tokenNetworkAddress).toBe(
      settlementConfig.ownTokenNetworks!['evm:base:31337']
    );
    expect(spspResponse.settlementTimeout).toBe(
      settlementConfig.settlementTimeout
    );
  });

  it('should open payment channel when chains intersect', async () => {
    const { event } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
      supportedChains: ['evm:base:31337'],
      settlementAddresses: {
        'evm:base:31337': '0xRequesterAddr1234567890abcdef1234567890abcd',
      },
    });
    const request = createPacketRequest(event);

    const result = await handler(createTestContext(request));
    expect(result.accept).toBe(true);
    if (!result.accept) throw new Error('unreachable');

    // Verify channel client was called to open a channel
    expect(mockChannelClient.openChannel).toHaveBeenCalledTimes(1);
    expect(mockChannelClient.openChannel).toHaveBeenCalledWith(
      expect.objectContaining({
        peerId: expect.stringContaining('nostr-'),
        chain: 'evm:base:31337',
        peerAddress: '0xRequesterAddr1234567890abcdef1234567890abcd',
        token: settlementConfig.ownPreferredTokens!['evm:base:31337'],
        tokenNetwork: settlementConfig.ownTokenNetworks!['evm:base:31337'],
      })
    );

    // Verify channelId is included in the SPSP response
    const responseEvent = decodeEventFromToon(
      Uint8Array.from(Buffer.from(result.data!, 'base64'))
    );
    const spspResponse = decryptSpspResponse(
      responseEvent,
      requesterSk,
      nodePubkey
    );
    expect(spspResponse.channelId).toBeDefined();
    expect(typeof spspResponse.channelId).toBe('string');
  });

  // ---------------------------------------------------------------------------
  // P1: NIP-44 encryption
  // ---------------------------------------------------------------------------

  it('should build NIP-44 encrypted response with SPSP fields', async () => {
    // The response event content must be NIP-44 encrypted for the requester.
    const { event, requestId } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
    });
    const request = createPacketRequest(event);

    const result = await handler(createTestContext(request));
    expect(result.accept).toBe(true);
    if (!result.accept) throw new Error('unreachable');

    const responseEvent = decodeEventFromToon(
      Uint8Array.from(Buffer.from(result.data!, 'base64'))
    );

    // The response should be tagged with the requester's pubkey
    const pTags = responseEvent.tags.filter((t) => t[0] === 'p');
    expect(pTags).toHaveLength(1);
    expect(pTags[0]![1]).toBe(requesterPubkey);

    // The response should reference the original request event
    const eTags = responseEvent.tags.filter((t) => t[0] === 'e');
    expect(eTags).toHaveLength(1);
    expect(eTags[0]![1]).toBe(event.id);

    // Verify that the content is encrypted (not plaintext JSON)
    expect(() => JSON.parse(responseEvent.content)).toThrow();

    // But the requester can decrypt it with their secret key
    const spspResponse = decryptSpspResponse(
      responseEvent,
      requesterSk,
      nodePubkey
    );
    expect(spspResponse.requestId).toBe(requestId);
    expect(spspResponse.destinationAccount).toBeDefined();
    expect(spspResponse.sharedSecret).toBeDefined();
  });

  // ---------------------------------------------------------------------------
  // P1: Graceful degradation
  // ---------------------------------------------------------------------------

  it('should gracefully degrade to basic SPSP response on settlement failure', async () => {
    // When channel opening fails, the handler should still return basic SPSP
    // parameters without settlement fields.
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

    const failingChannelClient = createMockChannelClient();
    (
      failingChannelClient.openChannel as ReturnType<typeof vi.fn>
    ).mockRejectedValue(new Error('Channel open timeout'));

    const failingHandler = createSpspHandshakeHandler({
      secretKey: nodeSk,
      ilpAddress: nodeIlpAddress,
      eventStore,
      settlementConfig,
      channelClient: failingChannelClient,
      adminClient: mockAdminClient,
    });

    const { event, requestId } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
      supportedChains: ['evm:base:31337'],
      settlementAddresses: {
        'evm:base:31337': '0xRequesterAddr1234567890abcdef1234567890abcd',
      },
    });
    const request = createPacketRequest(event);

    // Should NOT reject -- should gracefully degrade
    const result = await failingHandler(createTestContext(request));
    expect(result.accept).toBe(true);
    if (!result.accept) throw new Error('unreachable');

    // Decode and verify the response has basic SPSP fields but no settlement
    const responseEvent = decodeEventFromToon(
      Uint8Array.from(Buffer.from(result.data!, 'base64'))
    );
    const spspResponse = decryptSpspResponse(
      responseEvent,
      requesterSk,
      nodePubkey
    );

    // Basic SPSP fields must be present
    expect(spspResponse.requestId).toBe(requestId);
    expect(spspResponse.destinationAccount).toBeDefined();
    expect(spspResponse.sharedSecret).toBeDefined();

    // Settlement fields should be absent (graceful degradation)
    expect(spspResponse.negotiatedChain).toBeUndefined();
    expect(spspResponse.channelId).toBeUndefined();
    expect(spspResponse.settlementAddress).toBeUndefined();

    warnSpy.mockRestore();
  });

  // ---------------------------------------------------------------------------
  // P1: Peer registration after handshake
  // ---------------------------------------------------------------------------

  it('should register peer with connector after successful handshake', async () => {
    // After a successful SPSP handshake, the handler should register the
    // requester as a peer with the connector admin API.

    // First, store a kind:10032 event for the requester so the handler
    // can look up their BTP endpoint for peer registration.
    const requesterIlpInfo = {
      ilpAddress: 'g.test.requester',
      btpEndpoint: 'ws://requester-node:3000', // nosemgrep: detect-insecure-websocket
      assetCode: 'USD',
      assetScale: 6,
    };
    const peerInfoEvent = finalizeEvent(
      {
        kind: 10032,
        content: JSON.stringify(requesterIlpInfo),
        tags: [],
        created_at: Math.floor(Date.now() / 1000),
      },
      requesterSk
    );
    eventStore.store(peerInfoEvent);

    // Now send the SPSP request
    const { event } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
      supportedChains: ['evm:base:31337'],
      settlementAddresses: {
        'evm:base:31337': '0xRequesterAddr1234567890abcdef1234567890abcd',
      },
    });
    const request = createPacketRequest(event);

    const result = await handler(createTestContext(request));
    expect(result.accept).toBe(true);

    // Verify the admin client was called to register the peer
    expect(mockAdminClient.addPeer).toHaveBeenCalledTimes(1);
    expect(mockAdminClient.addPeer).toHaveBeenCalledWith(
      expect.objectContaining({
        id: expect.stringContaining('nostr-'),
        url: 'ws://requester-node:3000', // nosemgrep: detect-insecure-websocket
        routes: expect.arrayContaining([
          expect.objectContaining({
            prefix: 'g.test.requester',
          }),
        ]),
      })
    );
  });

  // ---------------------------------------------------------------------------
  // Coverage gap tests: AC #1 -- Response structure and edge cases
  // ---------------------------------------------------------------------------

  it('should return default-fulfillment in accept response', async () => {
    // Verifies the exact fulfillment value is 'default-fulfillment' as required
    // by the handler's direct response pattern (bypasses ctx.accept()).
    const { event } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
    });
    const request = createPacketRequest(event);

    const result = await handler(createTestContext(request));

    expect(result.accept).toBe(true);
    if (!result.accept) throw new Error('unreachable');
    expect(result.fulfillment).toBe('default-fulfillment');
  });

  it('should work without settlement config', async () => {
    // When no settlementConfig is provided, the handler should still process
    // the SPSP request and return a basic response with no settlement fields.
    const minimalHandler = createSpspHandshakeHandler({
      secretKey: nodeSk,
      ilpAddress: nodeIlpAddress,
      eventStore,
      // No settlementConfig, no channelClient, no adminClient
    });

    const { event, requestId } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
      supportedChains: ['evm:base:31337'],
      settlementAddresses: {
        'evm:base:31337': '0xRequesterAddr1234567890abcdef1234567890abcd',
      },
    });
    const request = createPacketRequest(event);

    const result = await minimalHandler(createTestContext(request));

    expect(result.accept).toBe(true);
    if (!result.accept) throw new Error('unreachable');
    expect(result.data).toBeDefined();

    // Decode and verify basic SPSP fields present, no settlement fields
    const responseEvent = decodeEventFromToon(
      Uint8Array.from(Buffer.from(result.data!, 'base64'))
    );
    const spspResponse = decryptSpspResponse(
      responseEvent,
      requesterSk,
      nodePubkey
    );

    expect(spspResponse.requestId).toBe(requestId);
    expect(spspResponse.destinationAccount).toMatch(/^g\.test\.node\.spsp\./);
    expect(spspResponse.sharedSecret).toBeDefined();
    expect(spspResponse.negotiatedChain).toBeUndefined();
    expect(spspResponse.channelId).toBeUndefined();
  });

  it('should return basic SPSP response when request has no supportedChains', async () => {
    // When the SPSP request does not include supportedChains, the handler
    // should skip settlement negotiation entirely and return basic params.
    const { event, requestId } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
      // No supportedChains, no settlementAddresses
    });
    const request = createPacketRequest(event);

    const result = await handler(createTestContext(request));

    expect(result.accept).toBe(true);
    if (!result.accept) throw new Error('unreachable');

    const responseEvent = decodeEventFromToon(
      Uint8Array.from(Buffer.from(result.data!, 'base64'))
    );
    const spspResponse = decryptSpspResponse(
      responseEvent,
      requesterSk,
      nodePubkey
    );

    // Basic SPSP fields present
    expect(spspResponse.requestId).toBe(requestId);
    expect(spspResponse.destinationAccount).toBeDefined();
    expect(spspResponse.sharedSecret).toBeDefined();

    // No settlement negotiation attempted
    expect(spspResponse.negotiatedChain).toBeUndefined();
    expect(spspResponse.channelId).toBeUndefined();
    expect(spspResponse.settlementAddress).toBeUndefined();

    // Channel client should NOT have been called
    expect(mockChannelClient.openChannel).not.toHaveBeenCalled();
  });

  it('should return basic SPSP response when chains do not overlap', async () => {
    // When the requester's supported chains have no overlap with the node's
    // chains, negotiateAndOpenChannel returns null and the response should
    // contain only basic SPSP fields (no settlement).
    const { event, requestId } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
      supportedChains: ['evm:polygon:137'], // Node only supports evm:base:31337
      settlementAddresses: {
        'evm:polygon:137': '0xRequesterAddr1234567890abcdef1234567890abcd',
      },
    });
    const request = createPacketRequest(event);

    const result = await handler(createTestContext(request));

    expect(result.accept).toBe(true);
    if (!result.accept) throw new Error('unreachable');

    const responseEvent = decodeEventFromToon(
      Uint8Array.from(Buffer.from(result.data!, 'base64'))
    );
    const spspResponse = decryptSpspResponse(
      responseEvent,
      requesterSk,
      nodePubkey
    );

    // Basic SPSP fields present
    expect(spspResponse.requestId).toBe(requestId);
    expect(spspResponse.destinationAccount).toBeDefined();
    expect(spspResponse.sharedSecret).toBeDefined();

    // Settlement fields absent (no chain overlap)
    expect(spspResponse.negotiatedChain).toBeUndefined();
    expect(spspResponse.channelId).toBeUndefined();
    expect(spspResponse.settlementAddress).toBeUndefined();
  });

  // ---------------------------------------------------------------------------
  // Coverage gap tests: AC #2 -- Graceful degradation warning log
  // ---------------------------------------------------------------------------

  it('should log warning when settlement negotiation fails', async () => {
    // Verifies that console.warn is called when settlement negotiation
    // throws, as required by AC #2 (graceful degradation with warning).
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

    const failingChannelClient = createMockChannelClient();
    (
      failingChannelClient.openChannel as ReturnType<typeof vi.fn>
    ).mockRejectedValue(new Error('Blockchain unavailable'));

    const failingHandler = createSpspHandshakeHandler({
      secretKey: nodeSk,
      ilpAddress: nodeIlpAddress,
      eventStore,
      settlementConfig,
      channelClient: failingChannelClient,
      adminClient: mockAdminClient,
    });

    const { event } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
      supportedChains: ['evm:base:31337'],
      settlementAddresses: {
        'evm:base:31337': '0xRequesterAddr1234567890abcdef1234567890abcd',
      },
    });
    const request = createPacketRequest(event);

    await failingHandler(createTestContext(request));

    // Verify console.warn was called with settlement failure message
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining('Settlement negotiation failed'),
      expect.stringContaining('Blockchain unavailable')
    );

    warnSpy.mockRestore();
  });

  // ---------------------------------------------------------------------------
  // Coverage gap tests: AC #3 -- Peer registration edge cases
  // ---------------------------------------------------------------------------

  it('should skip peer registration when no kind:10032 event exists in EventStore', async () => {
    // When the requester does not have a kind:10032 event stored, the handler
    // should still succeed but skip peer registration (no addPeer call).
    // EventStore is empty -- no kind:10032 event for the requester.
    const { event } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
    });
    const request = createPacketRequest(event);

    const result = await handler(createTestContext(request));

    expect(result.accept).toBe(true);
    // addPeer should NOT have been called (no kind:10032 to look up)
    expect(mockAdminClient.addPeer).not.toHaveBeenCalled();
  });

  it('should succeed without adminClient in config', async () => {
    // When no adminClient is provided, the handler should skip peer
    // registration entirely and still return a successful SPSP response.
    const noAdminHandler = createSpspHandshakeHandler({
      secretKey: nodeSk,
      ilpAddress: nodeIlpAddress,
      eventStore,
      settlementConfig,
      channelClient: mockChannelClient,
      // No adminClient
    });

    // Store a kind:10032 event that would trigger peer registration if
    // adminClient were present
    const peerInfoEvent = finalizeEvent(
      {
        kind: 10032,
        content: JSON.stringify({
          ilpAddress: 'g.test.requester',
          btpEndpoint: 'ws://requester-node:3000', // nosemgrep: detect-insecure-websocket
          assetCode: 'USD',
          assetScale: 6,
        }),
        tags: [],
        created_at: Math.floor(Date.now() / 1000),
      },
      requesterSk
    );
    eventStore.store(peerInfoEvent);

    const { event, requestId } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
    });
    const request = createPacketRequest(event);

    const result = await noAdminHandler(createTestContext(request));

    expect(result.accept).toBe(true);
    if (!result.accept) throw new Error('unreachable');

    // Verify the response is still valid
    const responseEvent = decodeEventFromToon(
      Uint8Array.from(Buffer.from(result.data!, 'base64'))
    );
    const spspResponse = decryptSpspResponse(
      responseEvent,
      requesterSk,
      nodePubkey
    );
    expect(spspResponse.requestId).toBe(requestId);
    expect(spspResponse.destinationAccount).toBeDefined();
    expect(spspResponse.sharedSecret).toBeDefined();
  });

  it('should skip peer registration when kind:10032 content has wrong shape', async () => {
    // When the kind:10032 event has valid JSON but is missing required fields
    // (btpEndpoint, ilpAddress), the handler should skip peer registration,
    // log a warning, and still return a successful SPSP response.
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

    // Store a kind:10032 event with valid JSON but missing btpEndpoint/ilpAddress
    const malformedPeerInfoEvent = finalizeEvent(
      {
        kind: 10032,
        content: JSON.stringify({ someField: 'value', assetCode: 'USD' }),
        tags: [],
        created_at: Math.floor(Date.now() / 1000),
      },
      requesterSk
    );
    eventStore.store(malformedPeerInfoEvent);

    const { event } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
    });
    const request = createPacketRequest(event);

    const result = await handler(createTestContext(request));

    expect(result.accept).toBe(true);
    // addPeer should NOT have been called (missing required fields)
    expect(mockAdminClient.addPeer).not.toHaveBeenCalled();
    // Should have logged a warning about missing fields
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining('missing required fields')
    );

    warnSpy.mockRestore();
  });

  it('should not reject when peer registration fails', async () => {
    // When adminClient.addPeer() throws, the handler should catch the error,
    // log a warning, and still return the successful SPSP response (non-fatal).
    const warnSpy = vi.spyOn(console, 'warn').mockImplementation(() => {});

    const failingAdminClient = createMockAdminClient();
    (failingAdminClient.addPeer as ReturnType<typeof vi.fn>).mockRejectedValue(
      new Error('Connector admin unreachable')
    );

    const failingPeerHandler = createSpspHandshakeHandler({
      secretKey: nodeSk,
      ilpAddress: nodeIlpAddress,
      eventStore,
      settlementConfig,
      channelClient: mockChannelClient,
      adminClient: failingAdminClient,
    });

    // Store kind:10032 so peer registration is attempted
    const peerInfoEvent = finalizeEvent(
      {
        kind: 10032,
        content: JSON.stringify({
          ilpAddress: 'g.test.requester',
          btpEndpoint: 'ws://requester-node:3000', // nosemgrep: detect-insecure-websocket
          assetCode: 'USD',
          assetScale: 6,
        }),
        tags: [],
        created_at: Math.floor(Date.now() / 1000),
      },
      requesterSk
    );
    eventStore.store(peerInfoEvent);

    const { event, requestId } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
    });
    const request = createPacketRequest(event);

    // Should NOT throw despite peer registration failure
    const result = await failingPeerHandler(createTestContext(request));

    expect(result.accept).toBe(true);
    if (!result.accept) throw new Error('unreachable');

    // The SPSP response should still be valid
    const responseEvent = decodeEventFromToon(
      Uint8Array.from(Buffer.from(result.data!, 'base64'))
    );
    const spspResponse = decryptSpspResponse(
      responseEvent,
      requesterSk,
      nodePubkey
    );
    expect(spspResponse.requestId).toBe(requestId);
    expect(spspResponse.destinationAccount).toBeDefined();
    expect(spspResponse.sharedSecret).toBeDefined();

    // Verify addPeer was attempted
    expect(failingAdminClient.addPeer).toHaveBeenCalledTimes(1);

    // Verify warning was logged
    expect(warnSpy).toHaveBeenCalledWith(
      expect.stringContaining('Peer registration failed'),
      expect.stringContaining('Connector admin unreachable')
    );

    warnSpy.mockRestore();
  });
});
