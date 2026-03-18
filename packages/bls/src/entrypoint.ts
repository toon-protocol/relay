/**
 * TOON Node with Bootstrap
 *
 * This entrypoint creates a complete TOON node with:
 * - BLS (Business Logic Server) for ILP packet handling
 * - Nostr Relay for peer discovery
 * - Bootstrap Service for automatic peer discovery
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
 * - DATA_DIR: Data directory for persistent storage
 */

import { serve } from '@hono/node-server';
import { Hono } from 'hono';
import type { Event as NostrEvent } from 'nostr-tools/pure';
import { BusinessLogicServer } from './bls/index.js';

/** Minimal interface for dynamically imported NIP34Handler */
interface NIP34HandlerLike {
  handleEvent(event: NostrEvent): Promise<{
    success: boolean;
    operation: string;
    message: string;
    metadata?: unknown;
  }>;
}
import { loadBlsConfigFromEnv } from './config.js';
import { ConfigError } from './errors.js';
import { PricingService } from './pricing/index.js';
import { createEventStore } from './storage/index.js';
import { encodeEventToToon, decodeEventFromToon } from './toon/index.js';
import { NostrRelayServer } from '@toon-protocol/relay';
import {
  BootstrapService,
  createDiscoveryTracker,
  ILP_PEER_INFO_KIND,
  type IlpPeerInfo,
  type SettlementConfig,
  buildIlpPeerInfoEvent,
  type HandlePacketRequest,
  type HandlePacketResponse,
  createHttpChannelClient,
  createHttpRuntimeClient as createHttpRuntimeClientV1,
  createHttpConnectorAdmin,
} from '@toon-protocol/core';
import { SimplePool } from 'nostr-tools/pool';

const BTP_SECRET = process.env['BTP_SECRET'] || 'toon-network-secret-2026';

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
  } = config;

  // secretKey not in BlsEnvConfig, load directly
  const secretKeyHex = process.env['NOSTR_SECRET_KEY'];
  if (!secretKeyHex) {
    throw new ConfigError(
      'NOSTR_SECRET_KEY',
      'Missing required environment variable'
    );
  }
  const secretKey = Uint8Array.from(Buffer.from(secretKeyHex, 'hex'));

  // Load TOON-specific config
  const connectorAdminUrl = process.env['CONNECTOR_ADMIN_URL'];
  const connectorUrl = process.env['CONNECTOR_URL'];
  const btpEndpoint = process.env['BTP_ENDPOINT'];
  const wsPort = parseInt(process.env['WS_PORT'] || '7100', 10);
  const bootstrapRelays = process.env['BOOTSTRAP_RELAYS']
    ? process.env['BOOTSTRAP_RELAYS'].split(',').filter((s) => s.trim())
    : [];
  let bootstrapPeers: string[] = [];
  let bootstrapPeerObjects: {
    pubkey: string;
    ilpAddress?: string;
    btpEndpoint?: string;
    relay?: string;
  }[] = [];
  if (process.env['BOOTSTRAP_PEERS']) {
    const raw = process.env['BOOTSTRAP_PEERS'].trim();
    if (raw.startsWith('[')) {
      // JSON array of peer objects
      try {
        bootstrapPeerObjects = JSON.parse(raw);
        bootstrapPeers = bootstrapPeerObjects.map((p) => p.pubkey);
      } catch {
        console.warn(
          '⚠️  Failed to parse BOOTSTRAP_PEERS JSON, treating as comma-separated pubkeys'
        );
        bootstrapPeers = raw.split(',').filter((s) => s.trim());
      }
    } else {
      // Comma-separated pubkeys
      bootstrapPeers = raw.split(',').filter((s) => s.trim());
    }
  }

  // Validate required TOON config
  if (!connectorAdminUrl) {
    throw new ConfigError(
      'CONNECTOR_ADMIN_URL',
      'Missing required environment variable'
    );
  }
  if (!connectorUrl) {
    throw new ConfigError(
      'CONNECTOR_URL',
      'Missing required environment variable'
    );
  }
  if (!btpEndpoint) {
    throw new ConfigError(
      'BTP_ENDPOINT',
      'Missing required environment variable'
    );
  }

  console.log('🚀 Starting TOON Node with Bootstrap...\n');
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
  const settlementInfo: SettlementConfig = {
    supportedChains: ['evm:base:31337'],
    settlementAddresses: {
      'evm:base:31337': process.env['PEER_EVM_ADDRESS'] || '',
    },
    preferredTokens: {
      'evm:base:31337':
        process.env['M2M_TOKEN_ADDRESS'] ||
        '0x5FbDB2315678afecb367f032d93F642f64180aa3', // Mock USDC (Anvil deterministic address)
    },
    tokenNetworks: {
      'evm:base:31337':
        process.env['TOKEN_NETWORK_REGISTRY'] ||
        '0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512',
    },
  };

  // -------------------------------------------------------------------------
  // Create Connector Clients (HTTP Mode)
  // -------------------------------------------------------------------------
  // NOTE: Runtime client uses admin URL since /admin/ilp/send is on admin server
  // connectorAdminUrl is guaranteed non-null by the validation above
  const adminUrl = connectorAdminUrl as string;
  const runtimeClient = createHttpRuntimeClientV1(adminUrl);

  const connectorAdmin = createHttpConnectorAdmin(adminUrl, BTP_SECRET);

  // Create HTTP channel client for payment channel operations
  const channelClient = createHttpChannelClient(adminUrl);

  // -------------------------------------------------------------------------
  // Initialize NIP-34 Handler (Git Operations via Nostr)
  // -------------------------------------------------------------------------
  const forgejoUrl = process.env['FORGEJO_URL'];
  const forgejoToken = process.env['FORGEJO_TOKEN'];
  const forgejoOwner = process.env['FORGEJO_OWNER'];

  let nip34Handler: NIP34HandlerLike | undefined;
  if (forgejoUrl && forgejoToken && forgejoOwner) {
    try {
      const { NIP34Handler } = await import('@toon-protocol/core/nip34');
      nip34Handler = new NIP34Handler({
        forgejoUrl,
        forgejoToken,
        defaultOwner: forgejoOwner,
        gitConfig: {
          userName: 'TOON Node',
          userEmail: `${nodeId}@toon.nostr`,
        },
        verbose: true,
      });
      console.log(`✅ NIP-34 Git integration enabled (Forgejo: ${forgejoUrl})`);
    } catch (error) {
      console.warn(
        '⚠️  Failed to initialize NIP-34 handler:',
        error instanceof Error ? error.message : error
      );
      console.warn('   NIP-34 Git integration will be disabled');
    }
  } else {
    console.log(
      '📝 NIP-34 Git integration disabled (set FORGEJO_URL, FORGEJO_TOKEN, FORGEJO_OWNER to enable)'
    );
  }

  // -------------------------------------------------------------------------
  // Create Packet Handler (BLS + NIP-34)
  // -------------------------------------------------------------------------
  const bls = new BusinessLogicServer(
    {
      basePricePerByte,
      pricingService,
      ownerPubkey,

      // NIP-34 event handler
      onNIP34Event: nip34Handler
        ? async (event) => {
            try {
              const result = await nip34Handler.handleEvent(event);

              if (result.success) {
                console.log(
                  `✅ NIP-34 ${result.operation}: ${result.message}`,
                  result.metadata || ''
                );
              } else {
                console.error(
                  `❌ NIP-34 ${result.operation}: ${result.message}`
                );
              }
            } catch (error) {
              console.error('❌ NIP-34 handler error:', error);
            }
          }
        : undefined,
    },
    eventStore
  );

  const handlePacket = async (
    request: HandlePacketRequest
  ): Promise<HandlePacketResponse> => {
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
  const blsHttpEndpoint =
    process.env['BLS_HTTP_ENDPOINT'] ||
    (process.env['NODE_ID']
      ? `http://toon-${process.env['NODE_ID']}:${blsPort}`
      : undefined);

  const ilpInfo: IlpPeerInfo = {
    ilpAddress,
    btpEndpoint,
    blsHttpEndpoint, // For bootstrap direct packet delivery
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
  const knownPeers =
    bootstrapPeerObjects.length > 0
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
  bootstrapService.setIlpClient(runtimeClient);
  bootstrapService.setConnectorAdmin(connectorAdmin);
  bootstrapService.setChannelClient(channelClient);

  // Create DiscoveryTracker
  const discoveryTracker = createDiscoveryTracker({
    secretKey,
    settlementInfo,
  });

  // Wire clients into discovery tracker
  discoveryTracker.setConnectorAdmin(connectorAdmin);

  // Listen to bootstrap events
  bootstrapService.on((event) => {
    console.log(`🔔 Bootstrap event: ${event.type}`, event);
  });

  discoveryTracker.on((event) => {
    console.log(`🔔 Discovery tracker event: ${event.type}`, event);
  });

  // Start bootstrap
  await bootstrapService.bootstrap();
  console.log(`✅ Bootstrap completed`);

  // If genesis peer (no bootstrap peers), publish own ILP info to local relay
  if (bootstrapPeers.length === 0) {
    const ilpInfoEvent = buildIlpPeerInfoEvent(ilpInfo, secretKey);
    eventStore.store(ilpInfoEvent);
    console.log(
      `✅ Genesis peer: Published ILP info (kind:10032) to local relay`
    );
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

  // Custom /handle-packet endpoint
  app.post('/handle-packet', async (c) => {
    const request = await c.req.json();
    const response = await handlePacket(request as HandlePacketRequest);
    // Feed accepted kind:10032 events to discovery tracker for peer discovery
    if (response.accept) {
      try {
        const toonBytes = Buffer.from(
          (request as HandlePacketRequest).data,
          'base64'
        );
        const decoded = decodeEventFromToon(toonBytes);
        if (decoded && decoded.kind === ILP_PEER_INFO_KIND) {
          discoveryTracker.processEvent(decoded);
        }
      } catch {
        /* decode failed, ignore */
      }
    }
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
  console.log('🎉 TOON node fully operational!\n');

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
