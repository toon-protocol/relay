/**
 * Tests for Story 2.2: SPSP Handshake Handler
 *
 * RED PHASE: All tests use describe.skip() because they import from
 * @crosstown/sdk which does not exist yet. The SDK will expose an
 * SpspHandshakeHandler that reimplements the SPSP request/response flow
 * as a kind-based handler (kind:23194), replacing the inline SPSP logic
 * in docker/src/entrypoint.ts.
 *
 * Infrastructure: Real NIP-44 encryption, real nostr-tools key generation,
 * real TOON codec. Connector transport is mocked.
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
  SqliteEventStore,
} from '@crosstown/relay';
import {
  buildSpspRequestEvent,
  SPSP_RESPONSE_KIND,
  type SpspResponse,
  type SettlementNegotiationConfig,
  type ConnectorChannelClient,
} from '@crosstown/core';

// --- RED PHASE: These imports will fail because @crosstown/sdk does not exist yet ---
// The SDK handler registers for kind:23194 and processes SPSP handshakes
// with settlement negotiation and payment channel opening.
import { createSpspHandshakeHandler } from '@crosstown/sdk';

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
function createMockAdminClient() {
  return {
    addPeer: vi.fn().mockResolvedValue(undefined),
    removePeer: vi.fn().mockResolvedValue(undefined),
  };
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
// Tests
// ============================================================================

/**
 * SKIP: @crosstown/sdk does not exist yet.
 *
 * These tests describe the SpspHandshakeHandler that will be created as part
 * of the SDK. The handler intercepts kind:23194 events from incoming ILP
 * packets, generates fresh SPSP parameters, optionally negotiates settlement
 * chains, opens payment channels, and returns an encrypted kind:23195 response.
 */
describe.skip('SpspHandshakeHandler', () => {
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
  let handler: ReturnType<typeof createSpspHandshakeHandler>;

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

    // Create the SDK handler under test
    // WILL FAIL: createSpspHandshakeHandler does not exist yet
    handler = createSpspHandshakeHandler({
      secretKey: nodeSk,
      ilpAddress: nodeIlpAddress,
      eventStore,
      toonEncoder: encodeEventToToon,
      toonDecoder: decodeEventFromToon,
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
    // FAILS because createSpspHandshakeHandler is not implemented.
    // Builds a real NIP-44 encrypted SPSP request and verifies the handler
    // returns an accept response with encrypted SPSP response data.
    const { event, requestId } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
    });
    const request = createPacketRequest(event);

    const result = await handler(request);

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
    // FAILS because the SDK handler does not exist yet.
    // Each SPSP request must receive a unique destinationAccount and sharedSecret.
    const { event: event1 } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
    });
    const { event: event2 } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
    });

    const result1 = await handler(createPacketRequest(event1));
    const result2 = await handler(createPacketRequest(event2));

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
    // FAILS because settlement negotiation in the SDK handler is not implemented.
    // Both parties support evm:base:31337, so negotiation should succeed.
    const { event } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
      supportedChains: ['evm:base:31337'],
      settlementAddresses: {
        'evm:base:31337': '0xRequesterAddr1234567890abcdef1234567890abcd',
      },
    });
    const request = createPacketRequest(event);

    const result = await handler(request);
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
  });

  it('should open payment channel when chains intersect', async () => {
    // FAILS because the SDK handler channel opening is not implemented.
    const { event } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
      supportedChains: ['evm:base:31337'],
      settlementAddresses: {
        'evm:base:31337': '0xRequesterAddr1234567890abcdef1234567890abcd',
      },
    });
    const request = createPacketRequest(event);

    const result = await handler(request);
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
    // FAILS because the SDK handler NIP-44 encryption is not implemented.
    // The response event content must be NIP-44 encrypted for the requester.
    const { event, requestId } = createSpspRequest(requesterSk, nodePubkey, {
      ilpAddress: 'g.test.requester',
    });
    const request = createPacketRequest(event);

    const result = await handler(request);
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
    // FAILS because the SDK handler graceful degradation is not implemented.
    // When channel opening fails, the handler should still return basic SPSP
    // parameters without settlement fields.
    const failingChannelClient = createMockChannelClient();
    (
      failingChannelClient.openChannel as ReturnType<typeof vi.fn>
    ).mockRejectedValue(new Error('Channel open timeout'));

    const failingHandler = createSpspHandshakeHandler({
      secretKey: nodeSk,
      ilpAddress: nodeIlpAddress,
      eventStore,
      toonEncoder: encodeEventToToon,
      toonDecoder: decodeEventFromToon,
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
    const result = await failingHandler(request);
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
  });

  // ---------------------------------------------------------------------------
  // P1: Peer registration after handshake
  // ---------------------------------------------------------------------------

  it('should register peer with connector after successful handshake', async () => {
    // FAILS because the SDK handler peer registration is not implemented.
    // After a successful SPSP handshake, the handler should register the
    // requester as a peer with the connector admin API.

    // First, store a kind:10032 event for the requester so the handler
    // can look up their BTP endpoint for peer registration.
    const requesterIlpInfo = {
      ilpAddress: 'g.test.requester',
      btpEndpoint: 'ws://requester-node:3000',
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

    const result = await handler(request);
    expect(result.accept).toBe(true);

    // Verify the admin client was called to register the peer
    expect(mockAdminClient.addPeer).toHaveBeenCalledTimes(1);
    expect(mockAdminClient.addPeer).toHaveBeenCalledWith(
      expect.objectContaining({
        id: expect.stringContaining('nostr-'),
        url: 'ws://requester-node:3000',
        routes: expect.arrayContaining([
          expect.objectContaining({
            prefix: 'g.test.requester',
          }),
        ]),
      })
    );
  });
});
