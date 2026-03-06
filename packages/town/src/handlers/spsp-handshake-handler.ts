/**
 * SPSP handshake handler for @crosstown/town.
 *
 * Handles kind:23194 SPSP request events from incoming ILP packets. Performs
 * the full SPSP handshake flow: decrypt the NIP-44 encrypted request, generate
 * fresh SPSP parameters (destination account + shared secret), optionally
 * negotiate settlement chains and open payment channels, build an NIP-44
 * encrypted kind:23195 response, and register the peer with the connector.
 *
 * The handler returns { accept: true, fulfillment: 'default-fulfillment', data }
 * directly (bypasses ctx.accept()) because the TOON-encoded SPSP response must
 * be in the top-level `data` field for the connector to relay it back in the
 * ILP FULFILL packet.
 *
 * The SDK pipeline handles signature verification, pricing validation, and
 * self-write bypass BEFORE this handler is invoked.
 */

import type { EventStore } from '@crosstown/relay';
import type { Handler, HandlerContext, HandlerResponse } from '@crosstown/sdk';
import {
  parseSpspRequest,
  buildSpspResponseEvent,
  negotiateAndOpenChannel,
  ILP_PEER_INFO_KIND,
} from '@crosstown/core';
import type {
  SpspResponse,
  SettlementNegotiationConfig,
  ConnectorChannelClient,
  ConnectorAdminClient,
  IlpPeerInfo,
} from '@crosstown/core';
import { encodeEventToToon } from '@crosstown/core/toon';

/**
 * Configuration for the SPSP handshake handler.
 *
 * Unlike the event storage handler (which only needs eventStore), the SPSP
 * handler needs access to the node's secret key (for NIP-44 decrypt/encrypt),
 * the ILP address (for generating destination accounts), settlement config
 * (for chain negotiation), channel client (for opening channels), admin client
 * (for peer registration), and event store (for looking up kind:10032 peer info).
 */
export interface SpspHandshakeHandlerConfig {
  /** Node's secret key for NIP-44 decryption/signing. */
  secretKey: Uint8Array;
  /** Node's ILP address (base for SPSP destination accounts). */
  ilpAddress: string;
  /** Event store for looking up peer's kind:10032 ILP info. */
  eventStore: EventStore;
  /** Optional: chains/addresses/tokens for settlement negotiation. */
  settlementConfig?: SettlementNegotiationConfig;
  /** Optional: for opening payment channels via connector. */
  channelClient?: ConnectorChannelClient;
  /** Optional: for registering peers with the connector. */
  adminClient?: ConnectorAdminClient;
}

/**
 * Creates an SPSP handshake handler that processes kind:23194 events.
 *
 * The handler:
 * 1. Decodes the TOON payload via ctx.decode()
 * 2. Decrypts the NIP-44 encrypted SPSP request
 * 3. Generates fresh SPSP parameters (unique destination + shared secret)
 * 4. Optionally negotiates settlement chains and opens payment channels
 * 5. Builds an NIP-44 encrypted kind:23195 SPSP response
 * 6. Registers the peer with the connector (non-fatal)
 * 7. Returns { accept: true, fulfillment: 'default-fulfillment', data }
 *
 * @param config - Handler configuration.
 * @returns A handler function compatible with `node.on(SPSP_REQUEST_KIND, handler)`.
 */
export function createSpspHandshakeHandler(
  config: SpspHandshakeHandlerConfig
): Handler {
  const {
    secretKey,
    ilpAddress,
    eventStore,
    settlementConfig,
    channelClient,
    adminClient,
  } = config;

  return async (ctx: HandlerContext): Promise<HandlerResponse> => {
    // 1. Decode the TOON payload into a structured NostrEvent (kind:23194)
    const event = ctx.decode();

    // 2. Decrypt and parse the NIP-44 encrypted SPSP request
    const spspRequest = parseSpspRequest(event, secretKey, event.pubkey);

    // 3. Generate fresh SPSP parameters
    const destinationAccount = `${ilpAddress}.spsp.${crypto.randomUUID()}`;
    const sharedSecretBytes = crypto.getRandomValues(new Uint8Array(32));
    const sharedSecret = Buffer.from(sharedSecretBytes).toString('base64');

    // 4. Build base SPSP response
    const spspResponse: SpspResponse = {
      requestId: spspRequest.requestId,
      destinationAccount,
      sharedSecret,
    };

    // 5. Attempt settlement negotiation if configured and request has supportedChains
    if (
      settlementConfig &&
      channelClient &&
      spspRequest.supportedChains &&
      spspRequest.supportedChains.length > 0
    ) {
      try {
        const result = await negotiateAndOpenChannel({
          request: spspRequest,
          config: settlementConfig,
          channelClient,
          senderPubkey: event.pubkey,
        });

        if (result) {
          // Merge settlement fields into the SPSP response
          spspResponse.negotiatedChain = result.negotiatedChain;
          spspResponse.settlementAddress = result.settlementAddress;
          spspResponse.tokenAddress = result.tokenAddress;
          spspResponse.tokenNetworkAddress = result.tokenNetworkAddress;
          spspResponse.channelId = result.channelId;
          spspResponse.settlementTimeout = result.settlementTimeout;
        }
      } catch (err) {
        // Graceful degradation (AC #2): log warning and continue with basic response
        console.warn(
          'Settlement negotiation failed, continuing with basic SPSP response:',
          err instanceof Error ? err.message : String(err)
        );
      }
    }

    // 6. Build kind:23195 encrypted response event
    const responseEvent = buildSpspResponseEvent(
      spspResponse,
      event.pubkey,
      secretKey,
      event.id
    );

    // 7. Encode the response event to TOON and then base64
    const responseToonBytes = encodeEventToToon(responseEvent);
    const base64ResponseToon =
      Buffer.from(responseToonBytes).toString('base64');

    // 8. Attempt peer registration (AC #3) -- non-fatal, before return
    if (adminClient) {
      try {
        // Look up the requester's kind:10032 event from EventStore
        const peerInfoEvents = eventStore.query([
          {
            kinds: [ILP_PEER_INFO_KIND],
            authors: [event.pubkey],
            limit: 1,
          },
        ]);

        const peerInfoEvent = peerInfoEvents[0];
        if (peerInfoEvent) {
          let peerInfo: IlpPeerInfo;
          try {
            peerInfo = JSON.parse(peerInfoEvent.content) as IlpPeerInfo;
          } catch (parseErr) {
            console.warn(
              'Failed to parse peer info event:',
              parseErr instanceof Error ? parseErr.message : String(parseErr)
            );
            // Skip peer registration if content is malformed
            return {
              accept: true,
              fulfillment: 'default-fulfillment',
              data: base64ResponseToon,
            };
          }

          // Validate required IlpPeerInfo fields at runtime (type assertion
          // does not enforce this). Missing fields = skip registration.
          if (
            typeof peerInfo.btpEndpoint !== 'string' ||
            typeof peerInfo.ilpAddress !== 'string'
          ) {
            console.warn(
              'Cannot register peer: kind:10032 content missing required fields (btpEndpoint, ilpAddress)'
            );
            return {
              accept: true,
              fulfillment: 'default-fulfillment',
              data: base64ResponseToon,
            };
          }

          const btpUrl = peerInfo.btpEndpoint;
          // Validate BTP URL format before registering.
          // Sanitize btpUrl in log output to prevent log injection from
          // malicious kind:10032 content (replace control chars/newlines).
          // Protocol validation check (not creating an insecure connection).
          // BTP endpoints use plain WebSocket for local Docker-to-Docker connections.
          // nosemgrep: detect-insecure-websocket
          if (btpUrl.startsWith('ws://') || btpUrl.startsWith('wss://')) {
            const peerId = `nostr-${event.pubkey.slice(0, 16)}`;

            await adminClient.addPeer({
              id: peerId,
              url: btpUrl,
              authToken: '',
              routes: [{ prefix: peerInfo.ilpAddress }],
            });
          } else {
            const sanitized = btpUrl.replace(/[\n\r\t]/g, '');
            console.warn(
              `Cannot register peer: invalid BTP endpoint (got: ${sanitized})`
            );
          }
        }
      } catch (err) {
        // Peer registration is non-fatal: log and continue
        console.warn(
          'Peer registration failed:',
          err instanceof Error ? err.message : String(err)
        );
      }
    }

    // 9. Return the accept response with TOON-encoded SPSP response as data
    // NOTE: We bypass ctx.accept() because data must be a top-level field,
    // not nested in metadata. The connector relays this data back to the
    // sender in the ILP FULFILL packet.
    return {
      accept: true,
      fulfillment: 'default-fulfillment',
      data: base64ResponseToon,
    };
  };
}
