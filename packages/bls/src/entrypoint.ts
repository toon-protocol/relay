/**
 * Crosstown Node with Nostr SPSP Bootstrap
 *
 * This entrypoint creates a complete Crosstown node with:
 * - BLS (Business Logic Server) for ILP packet handling
 * - Nostr Relay for peer discovery
 * - Bootstrap Service for automatic peer discovery via Nostr SPSP
 * - Connector integration for ILP routing
 *
 * Environment Variables:
 * - NODE_ID: Unique node identifier
 * - NOSTR_SECRET_KEY: Hex-encoded Nostr secret key
 * - ILP_ADDRESS: ILP address for this node
 * - CONNECTOR_ADMIN_URL: Connector Admin API URL (e.g., http://connector:8081)
 * - CONNECTOR_URL: Connector health/packet URL (e.g., http://connector:8080)
 * - BTP_ENDPOINT: BTP WebSocket endpoint (e.g., ws://connector:3000)
 * - BLS_PORT: BLS HTTP port (default: 3100)
 * - WS_PORT: Nostr relay WebSocket port (default: 7100)
 * - BOOTSTRAP_RELAYS: Comma-separated relay URLs (e.g., "ws://peer1:7100,ws://peer2:7100")
 * - BOOTSTRAP_PEERS: Comma-separated peer pubkeys to bootstrap with
 * - BASE_PRICE_PER_BYTE: Base price per byte (default: 10)
 * - SPSP_MIN_PRICE: Minimum price for SPSP requests (genesis=0, joiners=undefined)
 * - DATA_DIR: Data directory for persistent storage
 */

import { serve } from '@hono/node-server';
import { Hono } from 'hono';
import { WebSocketServer } from 'ws';
import { getPublicKey, type Event as NostrEvent } from 'nostr-tools/pure';
import { BusinessLogicServer } from './bls/index.js';
import { loadBlsConfigFromEnv } from './config.js';
import { ConfigError } from './errors.js';
import { PricingService } from './pricing/index.js';
import { createEventStore } from './storage/index.js';
import { encodeEventToToon, decodeEventFromToon } from './toon/index.js';
import { NostrRelayServer } from '@crosstown/relay';
import {
  BootstrapService,
  RelayMonitor,
  type IlpPeerInfo,
  type SpspRequestSettlementInfo,
  parseSpspRequest,
  buildSpspResponseEvent,
  buildIlpPeerInfoEvent,
  type SpspResponse,
  SPSP_REQUEST_KIND,
  ILP_PEER_INFO_KIND,
  type HandlePacketRequest,
  type HandlePacketResponse,
  createHttpChannelClient,
  createHttpRuntimeClient as createHttpRuntimeClientV1,
  createHttpConnectorAdmin,
} from '@crosstown/core';
import { SimplePool } from 'nostr-tools/pool';

const BTP_SECRET = process.env['BTP_SECRET'] || 'crosstown-network-secret-2026';

async function main(): Promise<void> {
  // Load BLS config
  const config = loadBlsConfigFromEnv();
  const {
    nodeId,
    pubkey,
    ilpAddress,
    port: blsPort,
    basePricePerByte,
    ownerPubkey,
    dataDir,
    kindOverrides,
    spspMinPrice,
  } = config;

  // secretKey not in BlsEnvConfig, load directly
  const secretKeyHex = process.env['NOSTR_SECRET_KEY'];
  if (!secretKeyHex) {
    throw new ConfigError('NOSTR_SECRET_KEY', 'Missing required environment variable');
  }
  const secretKey = Uint8Array.from(Buffer.from(secretKeyHex, 'hex'));

  // Load Crosstown-specific config
  const connectorAdminUrl = process.env['CONNECTOR_ADMIN_URL'];
  const connectorUrl = process.env['CONNECTOR_URL'];
  const btpEndpoint = process.env['BTP_ENDPOINT'];
  const wsPort = parseInt(process.env['WS_PORT'] || '7100', 10);
  const bootstrapRelays = process.env['BOOTSTRAP_RELAYS']
    ? process.env['BOOTSTRAP_RELAYS'].split(',').filter((s) => s.trim())
    : [];
  let bootstrapPeers: string[] = [];
  let bootstrapPeerObjects: { pubkey: string; ilpAddress?: string; btpEndpoint?: string; relay?: string }[] = [];
  if (process.env['BOOTSTRAP_PEERS']) {
    const raw = process.env['BOOTSTRAP_PEERS'].trim();
    if (raw.startsWith('[')) {
      // JSON array of peer objects
      try {
        bootstrapPeerObjects = JSON.parse(raw);
        bootstrapPeers = bootstrapPeerObjects.map((p) => p.pubkey);
      } catch {
        console.warn('⚠️  Failed to parse BOOTSTRAP_PEERS JSON, treating as comma-separated pubkeys');
        bootstrapPeers = raw.split(',').filter((s) => s.trim());
      }
    } else {
      // Comma-separated pubkeys
      bootstrapPeers = raw.split(',').filter((s) => s.trim());
    }
  }

  // Validate required Crosstown config
  if (!connectorAdminUrl) {
    throw new ConfigError('CONNECTOR_ADMIN_URL', 'Missing required environment variable');
  }
  if (!connectorUrl) {
    throw new ConfigError('CONNECTOR_URL', 'Missing required environment variable');
  }
  if (!btpEndpoint) {
    throw new ConfigError('BTP_ENDPOINT', 'Missing required environment variable');
  }

  console.log('🚀 Starting Crosstown Node with Bootstrap...\n');
  console.log(`  Node ID:            ${nodeId}`);
  console.log(`  Pubkey:             ${pubkey}`);
  console.log(`  ILP Address:        ${ilpAddress}`);
  console.log(`  BLS Port:           ${blsPort}`);
  console.log(`  Nostr Relay Port:   ${wsPort}`);
  console.log(`  Connector Admin:    ${connectorAdminUrl}`);
  console.log(`  BTP Endpoint:       ${btpEndpoint}`);
  if (bootstrapRelays.length > 0) {
    console.log(`  Bootstrap Relays:   ${bootstrapRelays.join(', ')}`);
  }
  if (bootstrapPeers.length > 0) {
    console.log(`  Bootstrap Peers:    ${bootstrapPeers.length} peer(s)`);
  }
  if (spspMinPrice !== undefined) {
    console.log(`  SPSP Min Price:     ${spspMinPrice} (genesis peer)`);
  }
  console.log('');

  // -------------------------------------------------------------------------
  // Create Event Store
  // -------------------------------------------------------------------------
  const { eventStore, storageSummary } = createEventStore(dataDir);
  console.log(`📦 Storage: ${storageSummary}`);

  // -------------------------------------------------------------------------
  // Create Pricing Service
  // -------------------------------------------------------------------------
  const pricingService = new PricingService({
    basePricePerByte,
    kindOverrides,
  });

  // -------------------------------------------------------------------------
  // Create Settlement Info
  // -------------------------------------------------------------------------
  // TODO: Read from environment (EVM address, token address, chain ID)
  const settlementInfo: SpspRequestSettlementInfo = {
    ilpAddress,
    supportedChains: ['evm:base:31337'],
    settlementAddresses: {
      'evm:base:31337': process.env['PEER_EVM_ADDRESS'] || '',
    },
    preferredTokens: {
      'evm:base:31337': process.env['M2M_TOKEN_ADDRESS'] || '0x5FbDB2315678afecb367f032d93F642f64180aa3',
    },
    tokenNetworks: {
      'evm:base:31337': process.env['TOKEN_NETWORK_REGISTRY'] || '0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512',
    },
  };

  // -------------------------------------------------------------------------
  // Create Connector Clients (HTTP Mode)
  // -------------------------------------------------------------------------
  // NOTE: Runtime client uses admin URL since /admin/ilp/send is on admin server
  const runtimeClient = createHttpRuntimeClientV1(connectorAdminUrl!);

  const connectorAdmin = createHttpConnectorAdmin(connectorAdminUrl!, BTP_SECRET);

  // Create HTTP channel client for payment channel operations
  const channelClient = createHttpChannelClient(connectorAdminUrl!);

  // -------------------------------------------------------------------------
  // Initialize NIP-34 Handler (Git Operations via Nostr)
  // -------------------------------------------------------------------------
  const forgejoUrl = process.env['FORGEJO_URL'];
  const forgejoToken = process.env['FORGEJO_TOKEN'];
  const forgejoOwner = process.env['FORGEJO_OWNER'];

  let nip34Handler: any | undefined;
  if (forgejoUrl && forgejoToken && forgejoOwner) {
    try {
      const { NIP34Handler } = await import('@crosstown/core/nip34');
      nip34Handler = new NIP34Handler({
        forgejoUrl,
        forgejoToken,
        defaultOwner: forgejoOwner,
        gitConfig: {
          userName: 'Crosstown Node',
          userEmail: `${nodeId}@crosstown.nostr`,
        },
        verbose: true,
      });
      console.log(`✅ NIP-34 Git integration enabled (Forgejo: ${forgejoUrl})`);
    } catch (error) {
      console.warn('⚠️  Failed to initialize NIP-34 handler:', error instanceof Error ? error.message : error);
      console.warn('   NIP-34 Git integration will be disabled');
    }
  } else {
    console.log('📝 NIP-34 Git integration disabled (set FORGEJO_URL, FORGEJO_TOKEN, FORGEJO_OWNER to enable)');
  }

  // -------------------------------------------------------------------------
  // Create Packet Handler (BLS + SPSP Interceptor + NIP-34)
  // -------------------------------------------------------------------------
  const bls = new BusinessLogicServer(
    {
      basePricePerByte,
      pricingService,
      ownerPubkey,
      spspMinPrice,

      // NIP-34 event handler
      onNIP34Event: nip34Handler ? async (event) => {
        try {
          const result = await nip34Handler.handleEvent(event);

          if (result.success) {
            console.log(`✅ NIP-34 ${result.operation}: ${result.message}`, result.metadata || '');
          } else {
            console.error(`❌ NIP-34 ${result.operation}: ${result.message}`);
          }
        } catch (error) {
          console.error('❌ NIP-34 handler error:', error);
        }
      } : undefined,
    },
    eventStore
  );

  const handlePacket = async (request: HandlePacketRequest): Promise<HandlePacketResponse> => {
    // Decode packet to check if it's an SPSP request
    let event: NostrEvent;
    let toonBytes: Uint8Array;
    try {
      toonBytes = Uint8Array.from(Buffer.from(request.data, 'base64'));
      event = decodeEventFromToon(toonBytes);
    } catch {
      // Not a valid TOON event, pass to BLS
      return bls.handlePacket(request);
    }

    // Intercept SPSP requests (kind:23194)
    if (event.kind === SPSP_REQUEST_KIND) {
      console.log(`📨 SPSP request from ${event.pubkey.slice(0, 16)}...`);

      // Enforce pricing: use spspMinPrice if defined (including 0 for genesis peers)
      const calculatedPrice = BigInt(toonBytes.length) * basePricePerByte;
      const price = spspMinPrice !== undefined ? BigInt(spspMinPrice) : calculatedPrice;

      const amount = BigInt(request.amount);
      if (amount < price) {
        console.log(`❌ SPSP rejected: insufficient payment (${amount} < ${price})`);
        return {
          accept: false,
          code: 'F04',
          message: 'Insufficient payment amount',
        };
      }

      try {
        const spspRequest = parseSpspRequest(event, secretKey, event.pubkey);

        // Settlement info for the SPSP response (channel opening is requester's responsibility)
        const chain = settlementInfo.supportedChains?.[0] ?? '';
        const negotiatedChain = chain || undefined;
        const settlementAddress = settlementInfo.settlementAddresses?.[chain];
        const tokenAddress = settlementInfo.preferredTokens?.[chain];
        const tokenNetworkAddress = settlementInfo.tokenNetworks?.[chain];

        // Register the requesting peer on our connector for bidirectional routing
        if (connectorAdmin && spspRequest.ilpAddress) {
          try {
            const peerId = `nostr-${event.pubkey.slice(0, 16)}`;

            // Look up the peer's kind:10032 event to get their BTP endpoint
            const peerEvents = eventStore.query([{
              kinds: [ILP_PEER_INFO_KIND],
              authors: [event.pubkey],
              limit: 1,
            }]);

            let btpUrl = '';
            if (peerEvents.length > 0 && peerEvents[0]) {
              try {
                const peerInfo: IlpPeerInfo = JSON.parse(peerEvents[0].content);
                btpUrl = peerInfo.btpEndpoint;
              } catch (parseError) {
                console.warn(`⚠️  Failed to parse peer info event: ${parseError instanceof Error ? parseError.message : 'Unknown error'}`);
              }
            }

            // Only register if we have a valid BTP URL
            if (btpUrl && (btpUrl.startsWith('ws://') || btpUrl.startsWith('wss://'))) {
              await connectorAdmin.addPeer({
                id: peerId,
                url: btpUrl,
                authToken: '',
                routes: [{ prefix: spspRequest.ilpAddress }],
              });
              console.log(`✅ Registered peer ${peerId} (${spspRequest.ilpAddress}) at ${btpUrl}`);
            } else {
              console.warn(`⚠️  Cannot register peer ${peerId}: no valid BTP endpoint found`);
            }
          } catch (peerError) {
            console.warn(`⚠️  Failed to register peer after SPSP handshake: ${peerError instanceof Error ? peerError.message : 'Unknown error'}`);
          }
        }

        // Build SPSP response
        const response: SpspResponse = {
          requestId: spspRequest.requestId,
          destinationAccount: ilpAddress,
          sharedSecret: Buffer.from(crypto.getRandomValues(new Uint8Array(32))).toString('base64'),
          negotiatedChain,
          settlementAddress,
          tokenAddress,
          tokenNetworkAddress,
        };

        const responseEvent = buildSpspResponseEvent(
          response,
          event.pubkey,
          secretKey,
          event.id
        );

        // TOON-encode response
        const responseToonBytes = encodeEventToToon(responseEvent);
        const responseBase64 = Buffer.from(responseToonBytes).toString('base64');

        // Store SPSP request
        eventStore.store(event);

        console.log(`✅ SPSP response sent to ${event.pubkey.slice(0, 16)}...`);

        return {
          accept: true,
          fulfillment: Buffer.from(new Uint8Array(32)).toString('base64'),
          data: responseBase64,
        } as HandlePacketResponse & { data: string };
      } catch (error) {
        console.error(`❌ SPSP handler error:`, error);
        return {
          accept: false,
          code: 'T00',
          message: `SPSP handler error: ${error instanceof Error ? error.message : 'Unknown'}`,
        };
      }
    }

    // Not SPSP: delegate to BLS
    return bls.handlePacket(request);
  };

  // -------------------------------------------------------------------------
  // Create Nostr Relay
  // -------------------------------------------------------------------------
  const relay = new NostrRelayServer({ port: wsPort }, eventStore);
  await relay.start();
  console.log(`✅ Nostr relay started on port ${wsPort}`);

  // -------------------------------------------------------------------------
  // Create ILP Peer Info
  // -------------------------------------------------------------------------
  // Construct BLS HTTP endpoint from environment or infer from hostname
  const blsHttpEndpoint = process.env['BLS_HTTP_ENDPOINT'] ||
    (process.env['NODE_ID'] ? `http://crosstown-${process.env['NODE_ID']}:${blsPort}` : undefined);

  const ilpInfo: IlpPeerInfo = {
    ilpAddress,
    btpEndpoint,
    blsHttpEndpoint,  // For bootstrap direct packet delivery
    assetCode: 'USD',
    assetScale: 6,
    supportedChains: settlementInfo.supportedChains || [],
    settlementAddresses: settlementInfo.settlementAddresses || {},
    preferredTokens: settlementInfo.preferredTokens || {},
    tokenNetworks: settlementInfo.tokenNetworks || {},
  };

  // -------------------------------------------------------------------------
  // Create Bootstrap Service and Relay Monitor (HTTP Mode)
  // -------------------------------------------------------------------------
  const knownPeers = bootstrapPeerObjects.length > 0
    ? bootstrapPeerObjects.map((p) => ({
        pubkey: p.pubkey,
        relayUrl: p.relay || bootstrapRelays[0] || '',
        btpEndpoint: p.btpEndpoint || '',
      }))
    : bootstrapPeers.map((pubkey) => ({
        pubkey,
        relayUrl: bootstrapRelays[0] || '',
        btpEndpoint: '',
      }));

  const pool = new SimplePool();

  // Create BootstrapService
  const bootstrapService = new BootstrapService(
    {
      knownPeers,
      queryTimeout: 30_000,
      ardriveEnabled: false,
      defaultRelayUrl: bootstrapRelays[0] || `ws://127.0.0.1:${wsPort}`,
      settlementInfo,
      ownIlpAddress: ilpAddress,
      toonEncoder: encodeEventToToon,
      toonDecoder: decodeEventFromToon,
      basePricePerByte,
      btpSecret: BTP_SECRET,
    },
    secretKey,
    ilpInfo,
    pool
  );

  // Wire HTTP clients into bootstrap service
  bootstrapService.setAgentRuntimeClient(runtimeClient);
  bootstrapService.setConnectorAdmin(connectorAdmin);
  bootstrapService.setChannelClient(channelClient);

  // Create RelayMonitor
  const relayMonitor = new RelayMonitor(
    {
      relayUrl: bootstrapRelays[0] || `ws://127.0.0.1:${wsPort}`,
      secretKey,
      toonEncoder: encodeEventToToon,
      toonDecoder: decodeEventFromToon,
      basePricePerByte,
      settlementInfo,
      defaultTimeout: 30_000,
    },
    pool
  );

  // Wire HTTP clients into relay monitor
  relayMonitor.setAgentRuntimeClient(runtimeClient);
  relayMonitor.setConnectorAdmin(connectorAdmin);

  // Listen to bootstrap events
  bootstrapService.on((event) => {
    console.log(`🔔 Bootstrap event: ${event.type}`, event);
  });

  relayMonitor.on((event) => {
    console.log(`🔔 Relay monitor event: ${event.type}`, event);
  });

  // Start bootstrap
  await bootstrapService.bootstrap();
  console.log(`✅ Bootstrap completed`);

  // If genesis peer (no bootstrap peers), publish own ILP info to local relay
  if (bootstrapPeers.length === 0) {
    const ilpInfoEvent = buildIlpPeerInfoEvent(ilpInfo, secretKey);
    eventStore.store(ilpInfoEvent);
    console.log(`✅ Genesis peer: Published ILP info (kind:10032) to local relay`);
    console.log(`   Event ID: ${ilpInfoEvent.id}`);
  }

  // -------------------------------------------------------------------------
  // Start BLS HTTP Server
  // -------------------------------------------------------------------------
  const app = new Hono();

  app.get('/health', (c) => {
    return c.json({
      status: 'healthy',
      nodeId,
      pubkey,
      ilpAddress,
      timestamp: Date.now(),
    });
  });

  // Custom /handle-packet endpoint with SPSP interception
  app.post('/handle-packet', async (c) => {
    const request = await c.req.json();
    const response = await handlePacket(request as HandlePacketRequest);
    return c.json(response);
  });

  // Mount other BLS routes (except /handle-packet which we override above)
  // Note: This will mount /handle-packet from BLS too, but our route above takes precedence
  app.route('/', bls.getApp());

  const server = serve({
    fetch: app.fetch,
    port: blsPort,
  });

  console.log(`✅ BLS HTTP server started on port ${blsPort}`);
  console.log('');
  console.log('🎉 Crosstown node fully operational!\n');

  // -------------------------------------------------------------------------
  // Graceful Shutdown
  // -------------------------------------------------------------------------
  const shutdown = async () => {
    console.log('Shutting down...');
    server.close();
    pool.close([]);
    await relay.stop();
    eventStore.close();
    process.exit(0);
  };

  process.on('SIGINT', shutdown);
  process.on('SIGTERM', shutdown);
}

main().catch((error) => {
  console.error('Fatal error:', error);
  process.exit(1);
});
