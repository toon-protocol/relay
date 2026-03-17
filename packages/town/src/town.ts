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
 * ## Deployment Modes
 *
 * - **Embedded** (`connector`): Pass an `EmbeddableConnectorLike` directly.
 *   Zero-latency packet delivery via direct function calls.
 * - **Standalone** (`connectorUrl`): Connect to an external connector via HTTP.
 *   The connector runs as a separate process or container.
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
import { createX402Handler } from './handlers/x402-publish-handler.js';
import { createHealthResponse } from './health.js';
import {
  BootstrapService,
  createDiscoveryTracker,
  ILP_PEER_INFO_KIND,
  createHttpIlpClient,
  createHttpConnectorAdmin,
  createHttpChannelClient,
  createDirectIlpClient,
  createDirectConnectorAdmin,
  createDirectChannelClient,
  SocialPeerDiscovery,
  buildIlpPeerInfoEvent,
  resolveChainConfig,
  SeedRelayDiscovery,
  publishSeedRelayEntry,
  buildServiceDiscoveryEvent,
  VERSION,
} from '@crosstown/core';
import type { ServiceDiscoveryContent, SkillDescriptor } from '@crosstown/core';
import type {
  ConnectorChannelClient,
  BootstrapEvent,
  IlpPeerInfo,
  HandlePacketRequest,
  ConnectorAdminClient,
  IlpClient,
  SettlementConfig,
  EmbeddableConnectorLike,
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
import {
  createPublicClient,
  createWalletClient,
  defineChain,
  http,
} from 'viem';
import type { WalletClient, PublicClient } from 'viem';
import { privateKeyToAccount } from 'viem/accounts';

// ---------- SDK Pipeline Constants ----------
const MAX_PAYLOAD_BASE64_LENGTH = 1_048_576;

// ---------- Public Types ----------

/**
 * Configuration for starting a Crosstown relay node via `startTown()`.
 *
 * Exactly one of `mnemonic` or `secretKey` must be provided.
 * Exactly one of `connector` or `connectorUrl` must be provided.
 *
 * - **Embedded mode** (`connector`): Pass an `EmbeddableConnectorLike` directly
 *   for zero-latency packet delivery. No HTTP overhead.
 * - **Standalone mode** (`connectorUrl`): Connect to an external connector via
 *   HTTP. The connector runs as a separate process.
 */
export interface TownConfig {
  // --- Identity (exactly one required) ---

  /** 12-word or 24-word BIP-39 mnemonic phrase. */
  mnemonic?: string;
  /** 32-byte secp256k1 secret key. */
  secretKey?: Uint8Array;

  // --- Connector (exactly one required) ---

  /**
   * Embedded connector instance for zero-latency mode.
   * Exactly one of `connector` or `connectorUrl` is required.
   */
  connector?: EmbeddableConnectorLike;
  /**
   * External connector URL (e.g., "http://localhost:8080").
   * Exactly one of `connector` or `connectorUrl` is required.
   */
  connectorUrl?: string;
  /**
   * External connector admin URL (e.g., "http://localhost:8081").
   * Defaults to the connectorUrl with port incremented by 1.
   * Only used in standalone mode (with `connectorUrl`).
   */
  connectorAdminUrl?: string;

  // --- Network ---

  /** WebSocket relay port (default: 7100). */
  relayPort?: number;
  /** BLS HTTP server port (default: 3100). */
  blsPort?: number;
  /** ILP address for this node (default: g.crosstown.<pubkeyShort>). */
  ilpAddress?: string;
  /** BTP WebSocket endpoint (default: ws://localhost:3000). */
  btpEndpoint?: string;

  // --- Pricing ---

  /** Base price per byte in ILP units (default: 10n). */
  basePricePerByte?: bigint;
  /** Routing buffer percentage for x402 multi-hop overhead (default: 10). */
  routingBufferPercent?: number;

  // --- x402 ---

  /** Enable x402 /publish endpoint (default: false). */
  x402Enabled?: boolean;
  /** Facilitator EVM address for x402 payments. Defaults to the node's EVM address. */
  facilitatorAddress?: string;

  // --- Peers ---

  /** Known peers to bootstrap with. */
  knownPeers?: { pubkey: string; relayUrl: string; btpEndpoint: string }[];

  // --- Chain / Settlement ---

  /** Chain preset name (default: 'anvil'). See resolveChainConfig(). */
  chain?: string;
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

  // --- Discovery ---

  /** Discovery mode: 'seed-list' for production, 'genesis' for dev (default: 'genesis'). */
  discovery?: 'seed-list' | 'genesis';
  /** Public Nostr relay URLs for seed relay discovery (used when discovery: 'seed-list'). */
  seedRelays?: string[];
  /** Whether to publish this node as a seed relay entry (default: false). */
  publishSeedEntry?: boolean;
  /** External WebSocket URL of this relay (required if publishSeedEntry is true). */
  externalRelayUrl?: string;

  // --- DVM ---

  /**
   * Optional DVM skill descriptor to include in service discovery events.
   * When provided, the service discovery event will include the `skill` field.
   * Typically computed by `node.getSkillDescriptor()` from the SDK.
   */
  skill?: SkillDescriptor;

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
  /** Connector URL (standalone mode only). */
  connectorUrl?: string;
  /** Connector admin URL (standalone mode only). */
  connectorAdminUrl?: string;
  basePricePerByte: bigint;
  routingBufferPercent: number;
  x402Enabled: boolean;
  knownPeers: { pubkey: string; relayUrl: string; btpEndpoint: string }[];
  dataDir: string;
  devMode: boolean;
  ardriveEnabled: boolean;
  relayUrls: string[];
  assetCode: string;
  assetScale: number;
  /** Discovery mode: 'seed-list' for production, 'genesis' for dev. */
  discovery: 'seed-list' | 'genesis';
  /** Public Nostr relay URLs for seed relay discovery. */
  seedRelays: string[];
  /** Whether to publish this node as a seed relay entry. */
  publishSeedEntry: boolean;
  /** External WebSocket URL of this relay (for seed entry publishing). */
  externalRelayUrl?: string;
  /** Chain preset name (e.g., 'anvil', 'arbitrum-one'). */
  chain: string;
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

  /** Discovery mode used by this instance. */
  discoveryMode: 'seed-list' | 'genesis';
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
  // nosemgrep: javascript.lang.security.detect-insecure-websocket.detect-insecure-websocket -- validation check, not a connection
  if (!relayUrl.startsWith('ws://') && !relayUrl.startsWith('wss://')) {
    throw new Error(
      'Invalid relay URL -- must use WebSocket scheme (ws or wss)'
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
 * Supports two deployment modes:
 * - **Embedded** (`connector`): Pass an EmbeddableConnectorLike directly for
 *   zero-latency packet delivery. No HTTP overhead.
 * - **Standalone** (`connectorUrl`): Connect to an external connector via HTTP.
 *
 * @param config - Node configuration. One of `connector`/`connectorUrl` and one
 *   of `mnemonic`/`secretKey` are required; all other fields have defaults.
 * @returns A running TownInstance.
 * @throws If both or neither of mnemonic/secretKey are provided.
 * @throws If both or neither of connector/connectorUrl are provided.
 *
 * @example
 * ```typescript
 * // Standalone mode (external connector)
 * const town = await startTown({
 *   mnemonic: 'abandon abandon abandon ...',
 *   connectorUrl: 'http://localhost:8080',
 * });
 *
 * // Embedded mode (zero-latency)
 * const town = await startTown({
 *   mnemonic: 'abandon abandon abandon ...',
 *   connector: myConnectorNode,
 * });
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

  // --- 1b. Validate connector mode ---
  const hasConnector = config.connector !== undefined;
  const hasConnectorUrl = config.connectorUrl !== undefined;

  if (hasConnector && hasConnectorUrl) {
    throw new Error(
      'TownConfig: provide either connector or connectorUrl, not both'
    );
  }
  if (!hasConnector && !hasConnectorUrl) {
    throw new Error('TownConfig: one of connector or connectorUrl is required');
  }
  const embeddedMode = hasConnector;

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
  const connectorAdminUrl = connectorUrl
    ? (config.connectorAdminUrl ?? deriveAdminUrl(connectorUrl))
    : undefined;
  const basePricePerByte = config.basePricePerByte ?? 10n;
  const routingBufferPercent = config.routingBufferPercent ?? 10;
  const x402Enabled = config.x402Enabled ?? false;
  const knownPeers = [...(config.knownPeers ?? [])];
  const dataDir = config.dataDir ?? './data';
  const devMode = config.devMode ?? false;
  const ardriveEnabled = config.ardriveEnabled ?? false;
  const relayUrls = config.relayUrls ?? [`ws://localhost:${relayPort}`];
  const assetCode = config.assetCode ?? 'USD';
  const assetScale = config.assetScale ?? 6;
  const discovery = config.discovery ?? 'genesis';
  const seedRelays = config.seedRelays ?? [];
  const publishSeedEntryFlag = config.publishSeedEntry ?? false;
  const externalRelayUrl = config.externalRelayUrl;

  // --- 3b. Resolve chain preset early (needed for resolvedConfig and settlement) ---
  const chainConfig = resolveChainConfig(config.chain);
  const chainKey = `evm:base:${chainConfig.chainId}`;

  const resolvedConfig: ResolvedTownConfig = {
    relayPort,
    blsPort,
    ilpAddress,
    btpEndpoint,
    ...(connectorUrl && { connectorUrl }),
    ...(connectorAdminUrl && { connectorAdminUrl }),
    basePricePerByte,
    routingBufferPercent,
    x402Enabled,
    knownPeers,
    dataDir,
    devMode,
    ardriveEnabled,
    relayUrls,
    assetCode,
    assetScale,
    discovery,
    seedRelays,
    publishSeedEntry: publishSeedEntryFlag,
    ...(externalRelayUrl && { externalRelayUrl }),
    chain: chainConfig.name,
  };

  // --- 4. Create data directory ---
  mkdirSync(dataDir, { recursive: true });

  // --- 5. EventStore ---
  const dbPath = join(dataDir, 'events.db');
  const eventStore: EventStore = new SqliteEventStore(dbPath);

  // --- 5b. Auto-populate settlement defaults from chain preset ---

  // Auto-populate settlement fields from chain preset when not explicitly set.
  // Explicit config values always win over chain preset defaults.
  const effectiveChainRpcUrls = config.chainRpcUrls ?? {
    [chainKey]: chainConfig.rpcUrl,
  };
  const effectivePreferredTokens = config.preferredTokens ?? {
    [chainKey]: chainConfig.usdcAddress,
  };
  const effectiveTokenNetworks =
    config.tokenNetworks ??
    (chainConfig.tokenNetworkAddress
      ? { [chainKey]: chainConfig.tokenNetworkAddress }
      : undefined);

  // --- 6. Settlement configuration ---
  let channelClient: ConnectorChannelClient | undefined;
  let settlementInfo: SettlementConfig | undefined;

  const hasSettlement =
    effectiveChainRpcUrls || effectiveTokenNetworks || effectivePreferredTokens;

  if (hasSettlement) {
    const supportedChains = Array.from(
      new Set([
        ...Object.keys(effectiveChainRpcUrls ?? {}),
        ...Object.keys(effectiveTokenNetworks ?? {}),
        ...Object.keys(effectivePreferredTokens ?? {}),
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
      preferredTokens: effectivePreferredTokens,
      tokenNetworks: effectiveTokenNetworks,
    };

    if (embeddedMode) {
      const conn = config.connector as NonNullable<typeof config.connector>;
      if (conn.openChannel && conn.getChannelState) {
        channelClient = createDirectChannelClient(
          conn as Required<
            Pick<EmbeddableConnectorLike, 'openChannel' | 'getChannelState'>
          >
        );
      }
    } else {
      channelClient = createHttpChannelClient(connectorAdminUrl as string);
    }
  }

  // --- 7. Connector admin client ---
  const adminClient: ConnectorAdminClient = embeddedMode
    ? createDirectConnectorAdmin(
        config.connector as NonNullable<typeof config.connector>
      )
    : createHttpConnectorAdmin(connectorAdminUrl as string, '');

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
    return c.json(
      createHealthResponse({
        phase: bootstrapPhase,
        pubkey: identity.pubkey,
        ilpAddress,
        peerCount: discoveryTracker.getPeerCount() + peerCount,
        discoveredPeerCount: discoveryTracker.getDiscoveredCount(),
        channelCount,
        basePricePerByte,
        x402Enabled,
        chain: chainConfig.name,
      })
    );
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
        } catch {
          /* decode failed, ignore */
        }
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

  // --- 10b. ILP client (created before x402 handler so it can be wired in) ---
  const ilpClient: IlpClient = embeddedMode
    ? createDirectIlpClient(
        config.connector as NonNullable<typeof config.connector>,
        {
          toonDecoder: (bytes: Uint8Array) => decodeEventFromToon(bytes),
        }
      )
    : createHttpIlpClient(connectorAdminUrl as string);

  // --- 10c. viem clients for x402 settlement (conditional) ---
  let x402WalletClient: WalletClient | undefined;
  let x402PublicClient: PublicClient | undefined;

  if (x402Enabled) {
    // Derive EVM private key from node identity (same secp256k1 key)
    // Best-effort zeroing of intermediate Buffer; hex string is immutable
    // and cannot be zeroed (JS limitation, same as fromMnemonic pattern).
    let keyBuffer: Buffer | undefined;
    try {
      // Buffer.from(TypedArray) copies the data — identity.secretKey is not aliased.
      keyBuffer = Buffer.from(identity.secretKey);
      const privateKeyHex = `0x${keyBuffer.toString('hex')}` as `0x${string}`;
      const account = privateKeyToAccount(privateKeyHex);
      const viemChain = defineChain({
        id: chainConfig.chainId,
        name: chainConfig.name,
        nativeCurrency: { name: 'ETH', symbol: 'ETH', decimals: 18 },
        rpcUrls: { default: { http: [] } },
      });

      x402PublicClient = createPublicClient({
        chain: viemChain,
        transport: http(chainConfig.rpcUrl),
      });
      x402WalletClient = createWalletClient({
        account,
        chain: viemChain,
        transport: http(chainConfig.rpcUrl),
      });
    } catch (error: unknown) {
      throw new Error(
        `x402 initialization failed: could not derive EVM account from identity key: ${error instanceof Error ? error.message : String(error)}`
      );
    } finally {
      if (keyBuffer) {
        keyBuffer.fill(0);
      }
    }
  }

  // --- 10d. x402 /publish route ---
  const x402Handler = createX402Handler({
    x402Enabled,
    chainConfig,
    basePricePerByte,
    routingBufferPercent,
    facilitatorAddress: config.facilitatorAddress ?? identity.evmAddress,
    ownPubkey: identity.pubkey,
    devMode,
    eventStore,
    ilpClient,
    walletClient: x402WalletClient,
    publicClient: x402PublicClient,
  });

  // Register /publish for both GET and POST methods
  app.get('/publish', (c: Context) => x402Handler.handlePublish(c));
  app.post('/publish', (c: Context) => x402Handler.handlePublish(c));

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

  bootstrapService.setIlpClient(ilpClient);

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

  // In embedded mode, wire the packet handler directly to the connector.
  // Mutable ref for discovery tracker (created below, used in handler callback).
  const discoveryTrackerRef: {
    current?: ReturnType<typeof createDiscoveryTracker>;
  } = {};

  const connector = config.connector;
  if (embeddedMode && connector?.setPacketHandler) {
    connector.setPacketHandler(async (request) => {
      const result = await handlePacket(request as HandlePacketRequest);
      // Feed accepted kind:10032 events to discovery tracker
      if (result.accept && discoveryTrackerRef.current) {
        try {
          const toonBytes = Buffer.from(
            (request as HandlePacketRequest).data,
            'base64'
          );
          const decoded = decodeEventFromToon(toonBytes);
          if (decoded && decoded.kind === ILP_PEER_INFO_KIND) {
            discoveryTrackerRef.current.processEvent(decoded);
          }
        } catch {
          /* decode failed, ignore */
        }
      }
      return result;
    });
  }

  // Wait for connector health (standalone mode only).
  // In embedded mode, the connector is in-process — no health check needed.
  if (!embeddedMode) {
    // If the connector is unreachable, clean up already-started servers before
    // propagating the error so we don't leak listening ports.
    try {
      await waitForConnector(connectorUrl as string);
    } catch (connectorError: unknown) {
      blsServer.close();
      await wsRelay.stop();
      eventStore.close?.();
      throw connectorError;
    }
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
  // Wire discovery tracker ref for embedded mode packet handler
  discoveryTrackerRef.current = discoveryTracker;
  // discoveryTracker peer count is read live via getPeerCount() in the
  // health endpoint — no need for a separate counter here.

  // --- 13b. Seed Relay Discovery (when discovery: 'seed-list') ---
  // Runs before bootstrap to populate knownPeers from seed relay list.
  let seedRelayDiscovery: SeedRelayDiscovery | undefined;
  if (discovery === 'seed-list' && seedRelays.length > 0) {
    seedRelayDiscovery = new SeedRelayDiscovery({
      publicRelays: seedRelays,
    });

    try {
      const seedResult = await seedRelayDiscovery.discover();
      // Convert discovered peers to KnownPeer[] format and merge with config
      const seedPeers = seedResult.discoveredPeers
        .filter((info) => info.pubkey)
        .map((info) => ({
          pubkey: info.pubkey as string,
          relayUrl:
            seedResult.connectedUrls[0] ?? `ws://localhost:${relayPort}`,
          btpEndpoint: info.btpEndpoint,
        }));

      // Merge with existing knownPeers (config peers take priority)
      const existingPubkeys = new Set(knownPeers.map((p) => p.pubkey));
      for (const seedPeer of seedPeers) {
        if (!existingPubkeys.has(seedPeer.pubkey)) {
          knownPeers.push(seedPeer);
        }
      }

      console.log(
        `[Town] Seed relay discovery: found ${seedPeers.length} peers from ${seedResult.connectedUrls.length} seed relay(s)`
      );
    } catch (seedError: unknown) {
      const msg =
        seedError instanceof Error ? seedError.message : 'Unknown error';
      console.warn(`[Town] Seed relay discovery failed: ${msg}`);
      // Continue with any knownPeers from config
    }
  }

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

        ilpClient
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

    // Self-write: publish own kind:10035 (Service Discovery)
    try {
      const serviceDiscoveryContent: ServiceDiscoveryContent = {
        serviceType: 'relay',
        ilpAddress,
        pricing: {
          basePricePerByte: Number(basePricePerByte),
          currency: 'USDC',
        },
        supportedKinds: [1, 10032, 10035, 10036],
        capabilities: x402Enabled ? ['relay', 'x402'] : ['relay'],
        chain: chainConfig.name,
        version: VERSION,
      };

      // Only include x402 field when enabled (AC #3: omit entirely when disabled)
      if (x402Enabled) {
        serviceDiscoveryContent.x402 = {
          enabled: true,
          endpoint: '/publish',
        };
      }

      // Include skill descriptor when DVM capabilities are configured (Story 5.4)
      if (config.skill) {
        serviceDiscoveryContent.skill = config.skill;
      }

      const serviceDiscoveryEvent = buildServiceDiscoveryEvent(
        serviceDiscoveryContent,
        identity.secretKey
      );
      eventStore.store(serviceDiscoveryEvent);

      // Publish to peers via ILP (fire-and-forget, same pattern as kind:10032)
      const firstPeer = knownPeers[0];
      const genesisResult = results[0];
      if (firstPeer && genesisResult) {
        const genesisIlpAddress = genesisResult.peerInfo.ilpAddress;
        const sdToonBytes = encodeEventToToon(serviceDiscoveryEvent);
        const sdBase64Toon = Buffer.from(sdToonBytes).toString('base64');
        const sdIlpAmount = String(
          BigInt(sdToonBytes.length) * basePricePerByte
        );

        ilpClient
          .sendIlpPacket({
            destination: genesisIlpAddress,
            amount: sdIlpAmount,
            data: sdBase64Toon,
          })
          .catch((err: unknown) => {
            const msg = err instanceof Error ? err.message : 'Unknown';
            console.warn(
              '[Town] Failed to publish service discovery via ILP:',
              msg
            );
          });
      }
    } catch (error: unknown) {
      console.warn('[Town] Failed to publish service discovery:', error);
    }

    // Exclude already-bootstrapped peers from discovery
    const bootstrapPeerPubkeys = results.map((r) => r.knownPeer.pubkey);
    discoveryTracker.addExcludedPubkeys(bootstrapPeerPubkeys);
  } catch (error: unknown) {
    console.error('[Town] Bootstrap failed:', error);
  }

  // --- 13c. Publish seed relay entry (after bootstrap complete) ---
  if (publishSeedEntryFlag && !externalRelayUrl) {
    console.warn(
      '[Town] publishSeedEntry is true but externalRelayUrl is not set -- skipping seed relay entry publication'
    );
  }
  if (publishSeedEntryFlag && externalRelayUrl && seedRelays.length > 0) {
    publishSeedRelayEntry({
      secretKey: identity.secretKey,
      relayUrl: externalRelayUrl,
      publicRelays: seedRelays,
    })
      .then(({ publishedTo, eventId }) => {
        console.log(
          `[Town] Published seed relay entry to ${publishedTo} relay(s), eventId: ${eventId}`
        );
      })
      .catch((err: unknown) => {
        const msg = err instanceof Error ? err.message : 'Unknown error';
        console.warn(`[Town] Failed to publish seed relay entry: ${msg}`);
      });
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

      // Close seed relay discovery connections
      if (seedRelayDiscovery) {
        await seedRelayDiscovery.close();
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
    discoveryMode: discovery,
  };

  return instance;
}
