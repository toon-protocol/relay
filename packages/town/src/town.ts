/**
 * startTown() -- Programmatic API for starting a Crosstown relay node.
 *
 * This module wraps the same SDK components used by docker/src/entrypoint-town.ts
 * into a single function call with a typed configuration object. Both
 * `startTown()` and the Docker entrypoint compose the same pipeline:
 *
 *   Identity -> Verification -> Pricing -> HandlerRegistry -> BLS + Relay + Bootstrap
 *
 * The key difference is lifecycle management: the Docker entrypoint uses
 * process-level signals (SIGINT/SIGTERM), while `startTown()` returns a
 * `TownInstance` with an explicit `.stop()` method.
 *
 * ## External vs Embedded Connector
 *
 * The initial implementation requires `connectorUrl` -- the node connects to
 * an external connector via HTTP. Embedded connector mode (where `startTown()`
 * creates and manages its own connector process) is deferred to a future story.
 */

import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { serve, type ServerType } from '@hono/node-server';
import { Hono, type Context } from 'hono';
import {
  HandlerRegistry,
  createVerificationPipeline,
  createPricingValidator,
  createHandlerContext,
  fromMnemonic,
  fromSecretKey,
} from '@crosstown/sdk';
import type {
  HandlePacketAcceptResponse,
  HandlePacketRejectResponse,
  NodeIdentity,
} from '@crosstown/sdk';
import { createEventStorageHandler } from './handlers/event-storage-handler.js';
import {
  BootstrapService,
  createDiscoveryTracker,
  ILP_PEER_INFO_KIND,
  createHttpRuntimeClient,
  createHttpConnectorAdmin,
  createHttpChannelClient,
  SocialPeerDiscovery,
  buildIlpPeerInfoEvent,
} from '@crosstown/core';
import type {
  ConnectorChannelClient,
  BootstrapEvent,
  IlpPeerInfo,
  HandlePacketRequest,
  ConnectorAdminClient,
  AgentRuntimeClient,
  SettlementConfig,
} from '@crosstown/core';
import {
  shallowParseToon,
  decodeEventFromToon,
  encodeEventToToon,
} from '@crosstown/core/toon';
import {
  SqliteEventStore,
  NostrRelayServer,
  RelaySubscriber,
} from '@crosstown/relay';
import type { EventStore } from '@crosstown/relay';
import type { Filter } from 'nostr-tools/filter';

// ---------- SDK Pipeline Constants ----------
const MAX_PAYLOAD_BASE64_LENGTH = 1_048_576;

// ---------- Public Types ----------

/**
 * Configuration for starting a Crosstown relay node via `startTown()`.
 *
 * Exactly one of `mnemonic` or `secretKey` must be provided. All other fields
 * have sensible defaults.
 *
 * **Note:** `connectorUrl` is required for the initial implementation. The node
 * connects to an external ILP connector via HTTP. Embedded connector mode
 * (where `startTown()` manages its own connector) is deferred to a future story.
 */
export interface TownConfig {
  // --- Identity (exactly one required) ---

  /** 12-word or 24-word BIP-39 mnemonic phrase. */
  mnemonic?: string;
  /** 32-byte secp256k1 secret key. */
  secretKey?: Uint8Array;

  // --- Network ---

  /** WebSocket relay port (default: 7100). */
  relayPort?: number;
  /** BLS HTTP server port (default: 3100). */
  blsPort?: number;
  /** ILP address for this node (default: g.crosstown.<pubkeyShort>). */
  ilpAddress?: string;
  /** BTP WebSocket endpoint (default: ws://localhost:3000). */
  btpEndpoint?: string;
  /**
   * External connector URL (e.g., "http://localhost:8080").
   * **Required** for the initial implementation. Embedded connector mode
   * is deferred to a future story.
   */
  connectorUrl: string;
  /**
   * External connector admin URL (e.g., "http://localhost:8081").
   * Defaults to the connectorUrl with port incremented by 1.
   */
  connectorAdminUrl?: string;

  // --- Pricing ---

  /** Base price per byte in ILP units (default: 10n). */
  basePricePerByte?: bigint;

  // --- Peers ---

  /** Known peers to bootstrap with. */
  knownPeers?: { pubkey: string; relayUrl: string; btpEndpoint: string }[];

  // --- Settlement (all optional -- omit to disable settlement) ---

  /** Chain ID -> RPC URL mapping (e.g., { 'evm:base:31337': 'http://localhost:8545' }). */
  chainRpcUrls?: Record<string, string>;
  /** Chain ID -> TokenNetwork contract address. */
  tokenNetworks?: Record<string, string>;
  /** Chain ID -> preferred token address. */
  preferredTokens?: Record<string, string>;

  // --- Storage ---

  /** Data directory path (default: ./data). */
  dataDir?: string;

  // --- Development ---

  /** Enable dev mode (skip verification). Default: false. */
  devMode?: boolean;

  // --- Advanced ---

  /** Enable ArDrive peer lookup (default: false). */
  ardriveEnabled?: boolean;
  /** Public Nostr relay URLs for social discovery. */
  relayUrls?: string[];
  /** Asset code for ILP (default: 'USD'). */
  assetCode?: string;
  /** Asset scale for ILP (default: 6). */
  assetScale?: number;
}

/**
 * Resolved configuration with all defaults applied. All fields are non-optional
 * (ports, pricing, paths have been filled in).
 */
export interface ResolvedTownConfig {
  relayPort: number;
  blsPort: number;
  ilpAddress: string;
  btpEndpoint: string;
  connectorUrl: string;
  connectorAdminUrl: string;
  basePricePerByte: bigint;
  knownPeers: { pubkey: string; relayUrl: string; btpEndpoint: string }[];
  dataDir: string;
  devMode: boolean;
  ardriveEnabled: boolean;
  relayUrls: string[];
  assetCode: string;
  assetScale: number;
}

/**
 * A running Crosstown relay node instance returned by `startTown()`.
 *
 * Provides lifecycle control (stop), identity info, and bootstrap results.
 */
export interface TownInstance {
  /** Whether the relay is currently running. */
  isRunning(): boolean;

  /** Gracefully stop the relay and release all resources. */
  stop(): Promise<void>;

  /**
   * Subscribe to a remote Nostr relay. Received events are stored in the
   * Town's EventStore. Returns a handle for lifecycle management.
   *
   * @param relayUrl - WebSocket URL of the relay to subscribe to.
   * @param filter - Nostr filter (kinds, authors, etc.).
   * @returns A TownSubscription handle.
   * @throws If the town is not running.
   */
  subscribe(relayUrl: string, filter: Filter): TownSubscription;

  /** The node's Nostr x-only public key (64-char hex). */
  pubkey: string;

  /** The node's EVM address (0x-prefixed). */
  evmAddress: string;

  /** The resolved configuration with all defaults applied. */
  config: ResolvedTownConfig;

  /** Bootstrap results from the startup phase. */
  bootstrapResult: {
    peerCount: number;
    channelCount: number;
  };
}

/**
 * Handle for managing an outbound subscription to a remote Nostr relay.
 * Returned by `TownInstance.subscribe()`.
 */
export interface TownSubscription {
  /** Close the subscription and disconnect from the relay. */
  close(): void;
  /** The relay URL this subscription is connected to. */
  relayUrl: string;
  /** Whether this subscription is still active. */
  isActive(): boolean;
}

// ---------- Internal Helpers ----------

/**
 * Derive connector admin URL from the connector URL by incrementing the port.
 * e.g., http://localhost:8080 -> http://localhost:8081
 *
 * @internal Exported for unit testing only.
 */
export function deriveAdminUrl(connectorUrl: string): string {
  const parsed = new URL(connectorUrl);
  const port = parseInt(parsed.port || '8080', 10);
  parsed.port = String(port + 1);
  return parsed.toString().replace(/\/$/, '');
}

/**
 * Wait for the connector health endpoint to become available.
 */
async function waitForConnector(url: string, timeoutMs = 60000): Promise<void> {
  const healthUrl = `${url}/health`;
  const start = Date.now();

  while (Date.now() - start < timeoutMs) {
    try {
      const response = await fetch(healthUrl, {
        signal: AbortSignal.timeout(2000),
      });
      if (response.ok) return;
    } catch {
      // retry
    }
    await new Promise((resolve) => setTimeout(resolve, 2000));
  }

  throw new Error(
    `Connector health check timed out after ${timeoutMs}ms: ${url}`
  );
}

// ---------- Subscription Helper ----------

/**
 * Create a subscription to a remote Nostr relay, storing received events
 * in the local EventStore. Returns a TownSubscription handle.
 *
 * @internal Exported for unit testing only. Use `TownInstance.subscribe()` instead.
 */
export function createSubscription(
  relayUrl: string,
  filter: Filter,
  eventStore: EventStore,
  activeSubscriptions: Set<TownSubscription>
): TownSubscription {
  // Validate WebSocket URL scheme to provide clear errors and prevent
  // non-WebSocket URLs from reaching SimplePool (consistency with BTP URL
  // validation convention in project-context.md).
  if (!relayUrl.startsWith('ws://') && !relayUrl.startsWith('wss://')) {
    throw new Error(
      `Invalid relay URL: "${relayUrl}" -- must use ws:// or wss:// scheme`
    );
  }

  const subscriber = new RelaySubscriber(
    { relayUrls: [relayUrl], filter },
    eventStore
  );
  const handle = subscriber.start();

  let active = true;
  // Track last-seen timestamp for future reconnection with `since:` filter.
  // Currently unused -- SimplePool handles reconnection internally.
  // eslint-disable-next-line prefer-const -- will be reassigned in future story
  let _lastSeenTimestamp = 0;
  void _lastSeenTimestamp;

  const subscription: TownSubscription = {
    close() {
      if (!active) return;
      active = false;
      handle.unsubscribe();
      activeSubscriptions.delete(subscription);
    },
    relayUrl,
    isActive() {
      return active;
    },
  };

  activeSubscriptions.add(subscription);
  return subscription;
}

// ---------- Main API ----------

/**
 * Start a Crosstown relay node with the given configuration.
 *
 * Composes the full SDK pipeline (identity, verification, pricing, handlers)
 * and starts the relay WebSocket server, BLS HTTP server, bootstrap service,
 * and relay monitor. Returns a `TownInstance` for lifecycle management.
 *
 * @param config - Node configuration. `connectorUrl` and one of
 *   `mnemonic`/`secretKey` are required; all other fields have defaults.
 * @returns A running TownInstance.
 * @throws If both or neither of mnemonic/secretKey are provided.
 * @throws If connectorUrl is missing.
 *
 * @example
 * ```typescript
 * import { startTown } from '@crosstown/town';
 *
 * const town = await startTown({
 *   mnemonic: 'abandon abandon abandon ...',
 *   connectorUrl: 'http://localhost:8080',
 * });
 *
 * console.log(`Relay running on ws://localhost:${town.config.relayPort}`);
 * console.log(`Pubkey: ${town.pubkey}`);
 *
 * // Later...
 * await town.stop();
 * ```
 */
export async function startTown(config: TownConfig): Promise<TownInstance> {
  // --- 1. Validate identity ---
  const hasMnemonic = config.mnemonic !== undefined;
  const hasSecretKey = config.secretKey !== undefined;

  if (hasMnemonic && hasSecretKey) {
    throw new Error(
      'TownConfig: provide either mnemonic or secretKey, not both'
    );
  }
  if (!hasMnemonic && !hasSecretKey) {
    throw new Error('TownConfig: one of mnemonic or secretKey is required');
  }

  // --- 2. Derive identity ---
  const identity: NodeIdentity = hasMnemonic
    ? fromMnemonic(config.mnemonic as string)
    : fromSecretKey(config.secretKey as Uint8Array);

  // --- 3. Resolve config with defaults ---
  const relayPort = config.relayPort ?? 7100;
  const blsPort = config.blsPort ?? 3100;
  const pubkeyShort = identity.pubkey.slice(0, 16);
  const ilpAddress = config.ilpAddress ?? `g.crosstown.${pubkeyShort}`;
  const btpEndpoint = config.btpEndpoint ?? 'ws://localhost:3000';
  const connectorUrl = config.connectorUrl;
  const connectorAdminUrl =
    config.connectorAdminUrl ?? deriveAdminUrl(connectorUrl);
  const basePricePerByte = config.basePricePerByte ?? 10n;
  const knownPeers = config.knownPeers ?? [];
  const dataDir = config.dataDir ?? './data';
  const devMode = config.devMode ?? false;
  const ardriveEnabled = config.ardriveEnabled ?? false;
  const relayUrls = config.relayUrls ?? [`ws://localhost:${relayPort}`];
  const assetCode = config.assetCode ?? 'USD';
  const assetScale = config.assetScale ?? 6;

  const resolvedConfig: ResolvedTownConfig = {
    relayPort,
    blsPort,
    ilpAddress,
    btpEndpoint,
    connectorUrl,
    connectorAdminUrl,
    basePricePerByte,
    knownPeers,
    dataDir,
    devMode,
    ardriveEnabled,
    relayUrls,
    assetCode,
    assetScale,
  };

  // --- 4. Create data directory ---
  mkdirSync(dataDir, { recursive: true });

  // --- 5. EventStore ---
  const dbPath = join(dataDir, 'events.db');
  const eventStore: EventStore = new SqliteEventStore(dbPath);

  // --- 6. Settlement configuration ---
  let channelClient: ConnectorChannelClient | undefined;
  let settlementInfo: SettlementConfig | undefined;

  const hasSettlement =
    config.chainRpcUrls || config.tokenNetworks || config.preferredTokens;

  if (hasSettlement) {
    const supportedChains = Array.from(
      new Set([
        ...Object.keys(config.chainRpcUrls ?? {}),
        ...Object.keys(config.tokenNetworks ?? {}),
        ...Object.keys(config.preferredTokens ?? {}),
      ])
    );

    // Build settlement addresses from the identity's EVM address
    const settlementAddresses: Record<string, string> = {};
    for (const chain of supportedChains) {
      settlementAddresses[chain] = identity.evmAddress;
    }

    settlementInfo = {
      supportedChains,
      settlementAddresses,
      preferredTokens: config.preferredTokens,
      tokenNetworks: config.tokenNetworks,
    };

    channelClient = createHttpChannelClient(connectorAdminUrl);
  }

  // --- 7. Connector admin client ---
  const adminClient: ConnectorAdminClient = createHttpConnectorAdmin(
    connectorAdminUrl,
    ''
  );

  // --- 8. SDK Pipeline ---
  const verifier = createVerificationPipeline({ devMode });

  const pricer = createPricingValidator({
    basePricePerByte,
    ownPubkey: identity.pubkey,
  });

  const registry = new HandlerRegistry();
  registry.onDefault(createEventStorageHandler({ eventStore }));

  const toonDecoder = (toon: string) => {
    const bytes = Buffer.from(toon, 'base64');
    return decodeEventFromToon(bytes);
  };

  const handlePacket = async (
    request: HandlePacketRequest
  ): Promise<HandlePacketAcceptResponse | HandlePacketRejectResponse> => {
    // Stage 1: Size check
    if (request.data.length > MAX_PAYLOAD_BASE64_LENGTH) {
      return { accept: false, code: 'F08', message: 'Payload too large' };
    }

    // Stage 2: Shallow TOON parse
    const toonBytes = Buffer.from(request.data, 'base64');
    let meta;
    try {
      meta = shallowParseToon(toonBytes);
    } catch {
      return { accept: false, code: 'F06', message: 'Invalid TOON payload' };
    }

    // Stage 3: Schnorr verification
    const verifyResult = await verifier.verify(meta, request.data);
    if (!verifyResult.verified) {
      if (verifyResult.rejection) {
        return verifyResult.rejection;
      }
      return { accept: false, code: 'F06', message: 'Verification failed' };
    }

    // Stage 4: Pricing validation
    let amount: bigint;
    try {
      amount = BigInt(request.amount);
    } catch {
      return {
        accept: false,
        code: 'T00',
        message: 'Invalid payment amount',
      };
    }
    const priceResult = pricer.validate(meta, amount);
    if (!priceResult.accepted) {
      if (priceResult.rejection) {
        return priceResult.rejection;
      }
      return {
        accept: false,
        code: 'F04',
        message: 'Pricing validation failed',
      };
    }

    // Stage 5: Handler dispatch
    const ctx = createHandlerContext({
      toon: request.data,
      meta,
      amount,
      destination: request.destination,
      toonDecoder,
    });

    try {
      return await registry.dispatch(ctx);
    } catch (err: unknown) {
      const errMsg = err instanceof Error ? err.message : 'Unknown error';
      console.error('[Town] Handler dispatch failed:', errMsg);
      return { accept: false, code: 'T00', message: 'Internal error' };
    }
  };

  // --- 9. Bootstrap service setup ---
  const bootstrapService = new BootstrapService(
    {
      knownPeers,
      ardriveEnabled,
      defaultRelayUrl: `ws://localhost:${relayPort}`,
      ...(settlementInfo && { settlementInfo }),
      ownIlpAddress: ilpAddress,
      toonEncoder: encodeEventToToon,
      toonDecoder: decodeEventFromToon,
      basePricePerByte,
    },
    identity.secretKey,
    {
      ilpAddress,
      btpEndpoint,
      assetCode,
      assetScale,
    }
  );

  let peerCount = 0;
  let channelCount = 0;

  // --- 10. BLS HTTP Server ---
  const app = new Hono();
  app.get('/health', (c: Context) => {
    const bootstrapPhase = bootstrapService.getPhase();
    return c.json({
      status: 'healthy',
      pubkey: identity.pubkey,
      ilpAddress,
      timestamp: Date.now(),
      sdk: true,
      ...(bootstrapPhase && { bootstrapPhase }),
      ...(bootstrapPhase === 'ready' && {
        peerCount: discoveryTracker.getPeerCount() + peerCount,
        discoveredPeerCount: discoveryTracker.getDiscoveredCount(),
        channelCount,
      }),
    });
  });

  app.post('/handle-packet', async (c: Context) => {
    try {
      const body = (await c.req.json()) as HandlePacketRequest;
      if (
        body.amount === undefined ||
        body.amount === null ||
        body.destination === undefined ||
        body.destination === null ||
        body.data === undefined ||
        body.data === null
      ) {
        return c.json(
          { accept: false, code: 'F00', message: 'Missing required fields' },
          400
        );
      }
      const result = await handlePacket(body);
      // Feed accepted kind:10032 events to discovery tracker for peer discovery
      if (result.accept) {
        try {
          const toonBytes = Buffer.from(body.data, 'base64');
          const decoded = decodeEventFromToon(toonBytes);
          if (decoded && decoded.kind === ILP_PEER_INFO_KIND) {
            discoveryTracker.processEvent(decoded);
          }
        } catch { /* decode failed, ignore */ }
      }
      return c.json(result, result.accept ? 200 : 400);
    } catch (error: unknown) {
      // Log the full error server-side for debugging, but return a generic
      // message to the caller to avoid leaking internal details (CWE-209).
      console.error('[Town] handle-packet error:', error);
      return c.json(
        { accept: false, code: 'T00', message: 'Internal server error' },
        500
      );
    }
  });

  const blsServer: ServerType = serve({
    fetch: app.fetch,
    port: blsPort,
  });

  // --- 11. WebSocket Relay ---
  const wsRelay = new NostrRelayServer({ port: relayPort }, eventStore);
  await wsRelay.start();
  await new Promise((resolve) => setTimeout(resolve, 500));

  // --- 12. Running state ---
  let running = true;

  // --- 13. Bootstrap ---
  bootstrapService.setConnectorAdmin(adminClient);
  if (channelClient) {
    bootstrapService.setChannelClient(channelClient);
  }

  const agentRuntimeClient: AgentRuntimeClient =
    createHttpRuntimeClient(connectorAdminUrl);
  bootstrapService.setAgentRuntimeClient(agentRuntimeClient);

  bootstrapService.on((event: BootstrapEvent) => {
    switch (event.type) {
      case 'bootstrap:peer-registered':
        peerCount++;
        break;
      case 'bootstrap:channel-opened':
        channelCount++;
        break;
      case 'bootstrap:ready':
        // Phase update handled automatically
        break;
    }
  });

  // Wait for connector health.
  // If the connector is unreachable, clean up already-started servers before
  // propagating the error so we don't leak listening ports.
  try {
    await waitForConnector(connectorUrl);
  } catch (connectorError: unknown) {
    blsServer.close();
    await wsRelay.stop();
    eventStore.close?.();
    throw connectorError;
  }

  // Create DiscoveryTracker
  const discoveryTracker = createDiscoveryTracker({
    secretKey: identity.secretKey,
    settlementInfo,
  });
  discoveryTracker.setConnectorAdmin(adminClient);
  if (channelClient) {
    discoveryTracker.setChannelClient(channelClient);
  }
  // discoveryTracker peer count is read live via getPeerCount() in the
  // health endpoint — no need for a separate counter here.

  try {
    const results = await bootstrapService.bootstrap();

    // Self-write: publish own kind:10032
    const ownIlpInfo: IlpPeerInfo = {
      ilpAddress,
      btpEndpoint,
      assetCode,
      assetScale,
      ...(settlementInfo?.supportedChains && {
        supportedChains: settlementInfo.supportedChains,
      }),
      ...(settlementInfo?.settlementAddresses && {
        settlementAddresses: settlementInfo.settlementAddresses,
      }),
      ...(settlementInfo?.preferredTokens && {
        preferredTokens: settlementInfo.preferredTokens,
      }),
      ...(settlementInfo?.tokenNetworks && {
        tokenNetworks: settlementInfo.tokenNetworks,
      }),
    };

    try {
      const ilpInfoEvent = buildIlpPeerInfoEvent(
        ownIlpInfo,
        identity.secretKey
      );
      eventStore.store(ilpInfoEvent);

      // Publish to genesis relay via ILP if we have bootstrap peers
      const firstPeer = knownPeers[0];
      const genesisResult = results[0];
      if (firstPeer && genesisResult) {
        const genesisIlpAddress = genesisResult.peerInfo.ilpAddress;
        const toonBytes = encodeEventToToon(ilpInfoEvent);
        const base64Toon = Buffer.from(toonBytes).toString('base64');
        const ilpAmount = String(BigInt(toonBytes.length) * basePricePerByte);

        agentRuntimeClient
          .sendIlpPacket({
            destination: genesisIlpAddress,
            amount: ilpAmount,
            data: base64Toon,
          })
          .catch((err: unknown) => {
            const msg = err instanceof Error ? err.message : 'Unknown';
            console.warn('[Town] Failed to publish via ILP:', msg);
          });
      }
    } catch (error: unknown) {
      console.warn('[Town] Failed to publish ILP info:', error);
    }

    // Exclude already-bootstrapped peers from discovery
    const bootstrapPeerPubkeys = results.map((r) => r.knownPeer.pubkey);
    discoveryTracker.addExcludedPubkeys(bootstrapPeerPubkeys);
  } catch (error: unknown) {
    console.error('[Town] Bootstrap failed:', error);
  }

  // Social discovery
  const socialDiscovery = new SocialPeerDiscovery(
    { relayUrls },
    identity.secretKey
  );
  const socialSubscription = socialDiscovery.start();

  // --- 14. Outbound subscription tracking ---
  const activeSubscriptions = new Set<TownSubscription>();

  // --- 15. Build TownInstance ---
  const instance: TownInstance = {
    isRunning() {
      return running;
    },

    subscribe(subscribeRelayUrl: string, filter: Filter): TownSubscription {
      if (!running) {
        throw new Error('Cannot subscribe: town is not running');
      }

      return createSubscription(
        subscribeRelayUrl,
        filter,
        eventStore,
        activeSubscriptions
      );
    },

    async stop() {
      if (!running) return;
      running = false;

      // Close outbound subscriptions first
      for (const sub of activeSubscriptions) {
        sub.close();
      }
      activeSubscriptions.clear();

      if (socialSubscription) {
        socialSubscription.unsubscribe();
      }

      await wsRelay.stop();
      blsServer.close();

      // Close the EventStore (optional method on the EventStore interface)
      eventStore.close?.();
    },

    pubkey: identity.pubkey,
    evmAddress: identity.evmAddress,
    config: resolvedConfig,
    bootstrapResult: {
      peerCount,
      channelCount,
    },
  };

  return instance;
}
