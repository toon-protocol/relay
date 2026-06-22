/**
 * startRelay() -- Programmatic API for starting a TOON relay node.
 *
 * This module wraps the same SDK components used by docker/src/entrypoint-sdk.ts
 * into a single function call with a typed configuration object. Both
 * `startRelay()` and the Docker entrypoint compose the same pipeline:
 *
 *   Identity -> Verification -> Pricing -> HandlerRegistry -> BLS + Relay + Bootstrap
 *
 * The key difference is lifecycle management: the Docker entrypoint uses
 * process-level signals (SIGINT/SIGTERM), while `startRelay()` returns a
 * `RelayInstance` with an explicit `.stop()` method.
 *
 * ## Deployment Modes
 *
 * The town node ALWAYS runs an embedded `ConnectorNode` so that packets
 * destined for its own ILP address can be routed locally (the connector and
 * the BLS handler must share a process for the round-trip to work).
 *
 * - **Standalone embedded** (no `connectorUrl`): A self-routing embedded
 *   connector with no upstream peers. Useful for genesis nodes and tests.
 * - **Embedded with parent** (`connectorUrl` set): The embedded connector
 *   is configured with `connectorUrl` as a parent BTP peer, plus a self-route
 *   for local delivery and a default-route to the parent for everything else.
 * - **Pre-built embedded** (`connector`): Pass a fully constructed
 *   `EmbeddableConnectorLike`. The town does not modify it.
 *
 * `connector` and `connectorUrl` are mutually exclusive — provide at most one.
 *
 * - **Oblivious** (`obliviousMode: true`, default `false`): the relay runs as a
 *   payment-oblivious app behind an external terminator. No embedded connector
 *   is created; no x402/EIP-3009/ILP-settlement code runs. The node exposes
 *   `POST /write` (event-as-JSON), trusting injected `X-TOON-Payer`/`-Amount`/
 *   `-Chain` headers without re-validating payment. Free NIP-01 WS reads are
 *   unchanged. Mutually exclusive with `connector`/`connectorUrl`. The embedded
 *   modes above remain the DEFAULT and are unchanged when `obliviousMode` is
 *   false.
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
} from '@toon-protocol/sdk';
import type {
  HandlePacketAcceptResponse,
  HandlePacketRejectResponse,
  NodeIdentity,
} from '@toon-protocol/sdk';
import { createEventStorageHandler } from './handlers/event-storage-handler.js';
import { createX402Handler } from './handlers/x402-publish-handler.js';
import { createObliviousWriteHandler } from './handlers/oblivious-write-handler.js';
import { createHealthResponse } from './health.js';
import {
  BootstrapService,
  createDiscoveryTracker,
  ILP_PEER_INFO_KIND,
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
} from '@toon-protocol/core';
import type {
  ServiceDiscoveryContent,
  SkillDescriptor,
} from '@toon-protocol/core';
import type {
  ConnectorChannelClient,
  BootstrapEvent,
  IlpPeerInfo,
  HandlePacketRequest,
  ConnectorAdminClient,
  IlpClient,
  SettlementConfig,
  EmbeddableConnectorLike,
} from '@toon-protocol/core';
import {
  shallowParseToon,
  decodeEventFromToon,
  encodeEventToToon,
} from '@toon-protocol/core/toon';
import { SqliteEventStore } from '../storage/index.js';
import { NostrRelayServer } from '../websocket/index.js';
import { RelaySubscriber } from '../subscriber/index.js';
import type { EventStore } from '../storage/index.js';
import type { Filter } from 'nostr-tools/filter';
import {
  ConnectorNode,
  createLogger as createConnectorLogger,
} from '@toon-protocol/connector';
import type { ConnectorConfig } from '@toon-protocol/connector';
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
 * Configuration for starting a TOON relay node via `startRelay()`.
 *
 * Exactly one of `mnemonic` or `secretKey` must be provided.
 * `connector` and `connectorUrl` are mutually exclusive — provide at most one.
 *
 * - When neither is provided, a standalone embedded `ConnectorNode` is built
 *   with only a self-route (no upstream peers).
 * - When `connectorUrl` is set, the embedded connector is configured with
 *   that URL as a parent BTP peer plus a default-route to it. `ilpAddress`
 *   becomes REQUIRED in this mode and must fall under the parent's prefix
 *   (e.g. `g.townhouse.<self>`).
 * - When `connector` is set, the caller-supplied `EmbeddableConnectorLike`
 *   is used as-is; town does not configure peers, routes, or settlement on it.
 */
export interface RelayConfig {
  // --- Identity (exactly one required) ---

  /** 12-word or 24-word BIP-39 mnemonic phrase. */
  mnemonic?: string;
  /** 32-byte secp256k1 secret key. */
  secretKey?: Uint8Array;

  // --- Connector ---

  /**
   * Pre-built embedded connector. Mutually exclusive with `connectorUrl`.
   * When provided, town does not modify the connector — peers, routes, and
   * settlement are the caller's responsibility.
   */
  connector?: EmbeddableConnectorLike;
  /**
   * Pre-built EventStore. When provided, town uses it instead of constructing
   * the default file-backed `SqliteEventStore` under `dataDir`. Useful for
   * tests (inject an `InMemoryEventStore`) and for embedding the relay with a
   * shared store. The caller owns its lifecycle when supplied.
   */
  eventStore?: EventStore;
  /**
   * Parent connector BTP URL (e.g. `ws://apex.example:3001`). When set, the
   * embedded connector is built with this URL as a parent peer and a default
   * `g.` route to that peer; `ilpAddress` MUST also be set and fall under the
   * parent's prefix. Mutually exclusive with `connector`.
   */
  connectorUrl?: string;
  /** BTP peer id to use for the parent connector (default: `'apex'`). */
  parentPeerId?: string;
  /** BTP auth token for the parent peer (default: empty string -- no-auth). */
  parentAuthToken?: string;
  /** Stable nodeId for the embedded connector (default: `toon-<pubkeyShort>`). */
  nodeId?: string;

  /**
   * Run as a payment-oblivious relay (default `false`). When `true`, the relay
   * runs as a payment-oblivious app behind an external terminator: no embedded
   * connector is created and no x402/EIP-3009/ILP-settlement code runs. The
   * node exposes `POST /write` (event-as-JSON), trusting injected
   * `X-TOON-Payer`/`X-TOON-Amount`/`X-TOON-Chain` headers without re-validating
   * payment. Free NIP-01 WS reads are unchanged. Mutually exclusive with
   * `connector`/`connectorUrl`. Embedded modes remain the default and unchanged
   * when this is `false`. Overridable via the `TOON_OBLIVIOUS_MODE` env var.
   */
  obliviousMode?: boolean;

  /** BTP server port for the embedded connector (default: 3000). */
  btpServerPort?: number;

  /**
   * EVM private key for settlement infrastructure on the embedded connector.
   * If not set, the identity's secp256k1 key is reused.
   */
  settlementPrivateKey?: string;

  /**
   * EVM treasury address advertised to the parent connector for the
   * embedded-with-parent peer entry. The apex's PerPacketClaimService uses
   * this as the `peerAddress` when the apex opens a payment channel toward
   * this child. Only meaningful when `connectorUrl` is set. When omitted,
   * the parent peer entry has no `evmAddress` and the apex's channel-open
   * call must supply `peerAddress` explicitly.
   */
  parentEvmAddress?: string;

  // --- Network ---

  /** WebSocket relay port (default: 7100). */
  relayPort?: number;
  /** BLS HTTP server port (default: 3100). */
  blsPort?: number;
  /**
   * ILP address for this node. Default `g.toon.<pubkeyShort>` is used only
   * when no parent connector is configured. When `connectorUrl` is set this
   * field is REQUIRED and must fall under the parent's address prefix.
   */
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
  /**
   * Chain ID -> settlement (recipient) address advertised in kind:10032.
   *
   * By default every supported chain advertises the identity's EVM address.
   * That is wrong for non-EVM chains (e.g. `solana:devnet`), whose settlement
   * recipient must be a chain-native address (a base58 Solana pubkey). Provide
   * a per-chain override here to advertise a chain-native recipient; chains
   * absent from this map keep the EVM-address default.
   *
   * NOTE (Phase-2 Stage 2 gate): advertising a Solana recipient is necessary
   * but NOT sufficient for a settleable Solana loop — the client must also open
   * a real on-chain Solana payment-channel PDA and sign over that PDA. See the
   * Stage-2 PR description / gate report.
   */
  settlementAddresses?: Record<string, string>;

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

  // --- Transport Privacy ---

  /**
   * Ator hidden service configuration for the relay.
   *
   * When enabled, the relay binds to localhost only (ator handles inbound routing)
   * and publishes the `.anon` address in seed relay discovery events.
   *
   * - `enabled: false` (default): Relay binds to `0.0.0.0`, no privacy overlay.
   * - `enabled: true`: Relay binds to `127.0.0.1`, publishes `anonAddress` for discovery.
   */
  ator?: {
    enabled: boolean;
    /** The `.anon` hidden service address for this relay (e.g., "wss://abc123.anon:443"). */
    anonAddress?: string;
    /** SOCKS5 proxy URL for outbound connections (default: "socks5h://127.0.0.1:9050"). */
    socksProxy?: string;
  };

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

  // --- Fee Override ---

  /**
   * Fee per event in ILP units (overrides basePricePerByte when set).
   * When provided, sets basePricePerByte to this value. Used by the
   * Townhouse orchestrator via TOON_FEE_PER_EVENT env var.
   */
  feePerEvent?: number;

  /**
   * NIP-40 time-to-live for this node's kind:10032 announcement, in seconds
   * (default 3600). The node re-publishes its announcement at half this
   * interval so a live apex stays fresh while an offline one expires, letting
   * clients skip its unreachable BTP endpoint (issue #261). Set to 0 to disable
   * the expiration tag and the heartbeat (non-expiring announcement). Override
   * via the `TOON_ANNOUNCEMENT_TTL_SECONDS` env var.
   */
  announcementTtlSeconds?: number;
}

/**
 * Resolved configuration with all defaults applied. All fields are non-optional
 * (ports, pricing, paths have been filled in).
 */
export interface ResolvedRelayConfig {
  relayPort: number;
  blsPort: number;
  ilpAddress: string;
  btpEndpoint: string;
  /** Stable nodeId of the embedded connector. */
  nodeId: string;
  /** Parent connector URL when peering with one (omitted otherwise). */
  connectorUrl?: string;
  /** Parent BTP peer id (only meaningful when connectorUrl is set). */
  parentPeerId?: string;
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
  /** Whether the relay is running in payment-oblivious mode (no connector). */
  obliviousMode: boolean;
}

/**
 * A running TOON relay node instance returned by `startRelay()`.
 *
 * Provides lifecycle control (stop), identity info, and bootstrap results.
 */
export interface RelayInstance {
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
   * @returns A RelaySubscription handle.
   * @throws If the town is not running.
   */
  subscribe(relayUrl: string, filter: Filter): RelaySubscription;

  /** The node's Nostr x-only public key (64-char hex). */
  pubkey: string;

  /** The node's EVM address (0x-prefixed). */
  evmAddress: string;

  /** The resolved configuration with all defaults applied. */
  config: ResolvedRelayConfig;

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
 * Returned by `RelayInstance.subscribe()`.
 */
export interface RelaySubscription {
  /** Close the subscription and disconnect from the relay. */
  close(): void;
  /** The relay URL this subscription is connected to. */
  relayUrl: string;
  /** Whether this subscription is still active. */
  isActive(): boolean;
}

// ---------- Subscription Helper ----------

/**
 * Create a subscription to a remote Nostr relay, storing received events
 * in the local EventStore. Returns a RelaySubscription handle.
 *
 * @internal Exported for unit testing only. Use `RelayInstance.subscribe()` instead.
 */
export function createSubscription(
  relayUrl: string,
  filter: Filter,
  eventStore: EventStore,
  activeSubscriptions: Set<RelaySubscription>
): RelaySubscription {
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

  const subscription: RelaySubscription = {
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
 * Start a TOON relay node with the given configuration.
 *
 * Composes the full SDK pipeline (identity, verification, pricing, handlers)
 * and starts the relay WebSocket server, BLS HTTP server, bootstrap service,
 * and relay monitor. Returns a `RelayInstance` for lifecycle management.
 *
 * The town node ALWAYS runs an embedded `ConnectorNode`. Three configurations
 * are supported:
 * - No connector args: standalone embedded connector with self-route only.
 * - `connectorUrl`: embedded connector configured with that URL as a parent
 *   BTP peer plus a default `g.` route to it. `ilpAddress` is REQUIRED here.
 * - `connector`: pass a pre-built `EmbeddableConnectorLike`; town does not
 *   modify it.
 *
 * @param config - Node configuration. One of `mnemonic`/`secretKey` is required;
 *   `connector` and `connectorUrl` are mutually exclusive.
 * @returns A running RelayInstance.
 * @throws If both or neither of mnemonic/secretKey are provided.
 * @throws If both connector and connectorUrl are provided.
 * @throws If connectorUrl is set without an explicit ilpAddress.
 *
 * @example
 * ```typescript
 * // Standalone (no parent)
 * const town = await startRelay({ mnemonic: 'abandon ...' });
 *
 * // Embedded with parent
 * const town = await startRelay({
 *   mnemonic: 'abandon ...',
 *   connectorUrl: 'ws://apex.example:3001',
 *   parentPeerId: 'apex',
 *   parentAuthToken: '',
 *   ilpAddress: 'g.townhouse.alice',
 * });
 * ```
 */
export async function startRelay(config: RelayConfig): Promise<RelayInstance> {
  // --- 1. Validate identity ---
  const hasMnemonic = config.mnemonic !== undefined;
  const hasSecretKey = config.secretKey !== undefined;

  if (hasMnemonic && hasSecretKey) {
    throw new Error(
      'RelayConfig: provide either mnemonic or secretKey, not both'
    );
  }
  if (!hasMnemonic && !hasSecretKey) {
    throw new Error('RelayConfig: one of mnemonic or secretKey is required');
  }

  // --- 1b. Validate connector mode ---
  const hasConnector = config.connector !== undefined;
  const hasConnectorUrl = config.connectorUrl !== undefined;

  if (hasConnector && hasConnectorUrl) {
    throw new Error(
      'RelayConfig: provide either connector or connectorUrl, not both'
    );
  }

  // Oblivious mode: payment-oblivious relay behind an external terminator. No
  // embedded connector is created, so it is mutually exclusive with the
  // connector/connectorUrl embedded modes. Env wins only when neither is set.
  const obliviousMode =
    config.obliviousMode ?? process.env['TOON_OBLIVIOUS_MODE'] === 'true';

  if (obliviousMode && (hasConnector || hasConnectorUrl)) {
    throw new Error(
      'RelayConfig: obliviousMode is mutually exclusive with connector/connectorUrl ' +
        '(an oblivious relay runs no embedded connector)'
    );
  }

  // When peering with a parent, the operator MUST set ilpAddress so it falls
  // under the parent's prefix (e.g. g.townhouse.<self>). The default
  // g.toon.<pubkey> address would not be routable from the parent.
  if (hasConnectorUrl && config.ilpAddress === undefined) {
    throw new Error(
      'RelayConfig: ilpAddress is required when connectorUrl is set ' +
        '(must fall under the parent connector prefix, e.g. g.townhouse.<self>)'
    );
  }

  // --- 2. Derive identity ---
  const identity: NodeIdentity = hasMnemonic
    ? fromMnemonic(config.mnemonic as string)
    : fromSecretKey(config.secretKey as Uint8Array);

  // --- 3. Resolve config with defaults ---
  const relayPort = config.relayPort ?? 7100;
  const blsPort = config.blsPort ?? 3100;
  const pubkeyShort = identity.pubkey.slice(0, 16);
  const ilpAddress = config.ilpAddress ?? `g.toon.${pubkeyShort}`;
  // When no public BTP endpoint is configured (operator hasn't set one, or the
  // apex .anyone hostname isn't resolved yet), advertise an EMPTY btpEndpoint
  // rather than a loopback URL. A loopback default (`ws://localhost:3000`) leaks
  // into a network-visible kind:10032 and is unreachable from outside the Docker
  // network — clients that faithfully dial it fail (issue #259). An empty value
  // is rejected gracefully by client discovery instead of misdirecting it.
  const btpEndpoint = config.btpEndpoint ?? '';
  const nodeId = config.nodeId ?? `toon-${pubkeyShort}`;
  const parentPeerId = config.parentPeerId ?? 'apex';
  const parentAuthToken = config.parentAuthToken ?? '';
  const connectorUrl = config.connectorUrl;
  const basePricePerByte =
    config.feePerEvent !== undefined
      ? BigInt(config.feePerEvent)
      : (config.basePricePerByte ?? 10n);
  const routingBufferPercent = config.routingBufferPercent ?? 10;
  // x402 settles payments on-chain inside this process. In oblivious mode the
  // process is payment-oblivious (an external terminator gates writes), so
  // force x402 off regardless of what the caller passed.
  const x402Enabled = obliviousMode ? false : (config.x402Enabled ?? false);
  const knownPeers = [...(config.knownPeers ?? [])];
  const dataDir = config.dataDir ?? './data';
  const devMode = config.devMode ?? false;
  const ardriveEnabled = config.ardriveEnabled ?? false;
  const relayUrls = config.relayUrls ?? [`ws://localhost:${relayPort}`];
  const assetCode = config.assetCode ?? 'USD';
  const assetScale = config.assetScale ?? 6;
  const discovery = config.discovery ?? 'genesis';
  // NIP-40 TTL for the kind:10032 announcement (issue #261). Env wins over the
  // config field; default 1h. A non-finite/negative value falls back to the
  // default, and 0 disables expiration + the heartbeat (non-expiring event).
  const announcementTtlSeconds = (() => {
    const fromEnv = process.env['TOON_ANNOUNCEMENT_TTL_SECONDS'];
    const raw =
      fromEnv !== undefined && fromEnv !== ''
        ? Number(fromEnv)
        : config.announcementTtlSeconds;
    if (raw === undefined) return 3600;
    if (!Number.isFinite(raw) || raw < 0) return 3600;
    return Math.floor(raw);
  })();
  const seedRelays = config.seedRelays ?? [];
  const publishSeedEntryFlag = config.publishSeedEntry ?? false;
  // Use ator .anon address as externalRelayUrl when ator is enabled and no explicit URL set
  const externalRelayUrl =
    config.externalRelayUrl ??
    (config.ator?.enabled && config.ator.anonAddress
      ? config.ator.anonAddress
      : undefined);

  // --- 3b. Resolve chain preset early (needed for resolvedConfig and settlement) ---
  // Relay-only sentinel: when the operator (or the Townhouse network resolver in
  // `custom` mode with no EVM provider) sets the chain to `'none'`, the node runs
  // as a pure relay — no settlement chain is resolved, so no ethers provider is
  // ever constructed and the node connects straight to its parent connector.
  // `resolveChainConfig` reads `TOON_CHAIN` itself (env wins over the parameter),
  // so we mirror that precedence here to detect the sentinel before it throws on
  // an unknown chain name.
  // Oblivious mode forces relay-only: no settlement chain, no provider, no
  // channels — the process never touches the payment layer. The `'none'`
  // sentinel (or oblivious mode) selects the relay-only branch below.
  const requestedChain = process.env['TOON_CHAIN'] || config.chain;
  const relayOnly = requestedChain === 'none' || obliviousMode;
  if (relayOnly) {
    console.log('[Town] connector.relay_only', {
      reason: 'no settlement chain configured (chain=none)',
    });
  }
  const chainConfig = relayOnly
    ? {
        name: 'none',
        chainId: 0,
        rpcUrl: '',
        usdcAddress: '',
        tokenNetworkAddress: '',
        registryAddress: '',
      }
    : resolveChainConfig(config.chain);
  const chainKey = `evm:base:${chainConfig.chainId}`;

  const resolvedConfig: ResolvedRelayConfig = {
    relayPort,
    blsPort,
    ilpAddress,
    btpEndpoint,
    nodeId,
    ...(connectorUrl && { connectorUrl, parentPeerId }),
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
    obliviousMode,
  };

  // --- 3c. Auto-create embedded connector when no pre-built one was supplied ---
  // Skipped entirely in oblivious mode: a payment-oblivious relay runs no
  // embedded connector (an external terminator gates writes).
  let autoCreatedConnector: ConnectorNode | null = null;
  if (!hasConnector && !obliviousMode) {
    const btpServerPort = config.btpServerPort ?? 3000;
    const connectorLogger = createConnectorLogger(
      nodeId,
      (process.env['TOON_CONNECTOR_LOG_LEVEL'] as
        | 'debug'
        | 'info'
        | 'warn'
        | 'error'
        | undefined) ?? 'warn'
    );

    // Routes: always self-route for local delivery; add a parent default route
    // when peering. Local delivery is triggered by `nextHop === nodeId` (or
    // the literal 'local'); the connector's packet-handler.ts then auto-skips
    // settlement fees for local hops.
    const routes: {
      prefix: string;
      nextHop: string;
      priority?: number;
    }[] = [{ prefix: ilpAddress, nextHop: nodeId, priority: 100 }];

    // Peers: only the parent, when configured.
    const peers: {
      id: string;
      url: string;
      authToken: string;
      relation?: 'parent' | 'peer' | 'child';
      evmAddress?: string;
    }[] = [];

    if (hasConnectorUrl) {
      peers.push({
        id: parentPeerId,
        url: connectorUrl as string,
        authToken: parentAuthToken,
        // Tag the upstream as our PARENT so the embedded connector's
        // relation-aware logic applies (toon-protocol/connector#78): a child
        // skips the inbound per-packet-claim requirement for PREPAREs forwarded
        // by its parent (the parent settles in aggregate and attaches no
        // per-packet claim to a child). Without this the peer defaults to
        // 'peer' and the child F06-rejects every parent-forwarded paid packet.
        // NOTE: `parentPeerId` MUST equal the parent connector's nodeId (its BTP
        // auth identity), since the connector keys peerRelations by the
        // auth-declared peerId of the inbound session — not a local alias.
        relation: 'parent',
        // When the operator publishes their EVM treasury address to the
        // parent, the apex can open a settlement channel toward this child
        // without needing to discover the address via kind:10032. The
        // connector schema treats this field as optional metadata.
        ...(config.parentEvmAddress && { evmAddress: config.parentEvmAddress }),
      });
      // Connector's isValidILPAddress rejects trailing dots; RoutingTable
      // adds the delimiter at match time, so 'g' matches 'g.foo' correctly.
      routes.push({ prefix: 'g', nextHop: parentPeerId, priority: 0 });
    }

    // chainProviders entry — wires the embedded ConnectorNode's ClaimReceiver
    // so it can verify per-packet claims signed by the apex (image >=3.4.0).
    // Same secp256k1 key derives Nostr identity AND EVM treasury account.
    // We only build the entry when the chain preset has all settlement
    // addresses populated; presets like arbitrum-one have empty registry/
    // tokenNetwork strings, in which case we degrade gracefully (no warn-fail).
    const hasSettlementAddresses =
      !!chainConfig.rpcUrl &&
      !!chainConfig.registryAddress &&
      !!chainConfig.tokenNetworkAddress &&
      !!chainConfig.usdcAddress;

    let chainProvidersEntry: {
      chainType: 'evm';
      chainId: string;
      rpcUrl: string;
      registryAddress: string;
      tokenAddress: string;
      keyId: string;
    } | null = null;
    if (hasSettlementAddresses) {
      // chainId here is the connector's `evm:<numeric>` form, NOT the
      // chainKey ('evm:base:<numeric>') used for settlement maps.
      // When operator supplies `settlementPrivateKey`, prefer it over the
      // identity-derived hex — this lets the embedded connector's
      // ClaimReceiver use a funded EVM account (e.g. Anvil deterministic
      // privkey) distinct from the Nostr identity.
      const keyHex =
        config.settlementPrivateKey ??
        `0x${Buffer.from(identity.secretKey).toString('hex')}`;
      if (!/^0x[0-9a-fA-F]{64}$/.test(keyHex)) {
        throw new Error(
          `RelayConfig.settlementPrivateKey must be a 0x-prefixed 32-byte hex string (got length ${keyHex.length}); cannot wire chainProviders for ${chainConfig.name}`
        );
      }
      chainProvidersEntry = {
        chainType: 'evm',
        chainId: `evm:${chainConfig.chainId}`,
        rpcUrl: chainConfig.rpcUrl,
        registryAddress: chainConfig.registryAddress,
        tokenAddress: chainConfig.usdcAddress,
        keyId: keyHex,
      };
    } else {
      console.warn('[Town] connector.chain_providers_skipped', {
        chain: chainConfig.name,
        reason: 'missing settlement addresses',
      });
    }

    // eslint-disable-next-line @typescript-eslint/no-explicit-any
    const connectorConfig: any = {
      nodeId,
      btpServerPort,
      environment: 'development' as const,
      deploymentMode: 'embedded' as const,
      peers,
      routes,
      localDelivery: { enabled: false },
      // Children don't expose an admin API — the apex parent is the
      // operator-facing surface. Disabling avoids a hard runtime dep on
      // express in the town docker bundle.
      adminApi: { enabled: false },
      // Belt-and-braces: zero connector forwarding fee. Combined with the
      // packet-handler's automatic skip for local-delivery hops this keeps the
      // child fee surface flat regardless of peering topology.
      settlement: {
        connectorFeePercentage: 0,
      } as unknown as NonNullable<ConnectorConfig['settlement']>,
      ...(chainProvidersEntry && { chainProviders: [chainProvidersEntry] }),
    };
    // Ator/SOCKS5 transport propagation also covers the parent dial when
    // running inside a hidden-service deployment.
    if (config.ator?.enabled && config.ator.anonAddress) {
      connectorConfig.transport = {
        type: 'socks5',
        socksProxy: config.ator.socksProxy ?? 'socks5h://127.0.0.1:9050',
        externalUrl: config.ator.anonAddress,
        managed: false,
      };
    }
    autoCreatedConnector = new ConnectorNode(connectorConfig, connectorLogger);
  }

  // Effective connector: user-provided or auto-created. Null in oblivious mode
  // (no embedded connector); connector-dependent builders below are skipped.
  const effectiveConnector: EmbeddableConnectorLike | null =
    config.connector ??
    (autoCreatedConnector as unknown as EmbeddableConnectorLike | null);

  // --- 4. Create data directory ---
  mkdirSync(dataDir, { recursive: true });

  // --- 5. EventStore ---
  // Caller-supplied store wins (tests/embedding); otherwise default to the
  // file-backed SqliteEventStore under dataDir.
  const dbPath = join(dataDir, 'events.db');
  const eventStore: EventStore =
    config.eventStore ?? new SqliteEventStore(dbPath);

  // --- 5b. Auto-populate settlement defaults from chain preset ---

  // Auto-populate settlement fields from chain preset when not explicitly set.
  // Explicit config values always win over chain preset defaults.
  const effectiveChainRpcUrls =
    config.chainRpcUrls ??
    (relayOnly ? undefined : { [chainKey]: chainConfig.rpcUrl });
  const effectivePreferredTokens =
    config.preferredTokens ??
    (relayOnly ? undefined : { [chainKey]: chainConfig.usdcAddress });
  const effectiveTokenNetworks =
    config.tokenNetworks ??
    (chainConfig.tokenNetworkAddress
      ? { [chainKey]: chainConfig.tokenNetworkAddress }
      : undefined);

  // --- 6. Settlement configuration ---
  let channelClient: ConnectorChannelClient | undefined;
  let settlementInfo: SettlementConfig | undefined;

  const hasSettlement =
    effectiveChainRpcUrls ||
    effectiveTokenNetworks ||
    effectivePreferredTokens ||
    config.settlementAddresses;

  if (hasSettlement) {
    const supportedChains = Array.from(
      new Set([
        ...Object.keys(effectiveChainRpcUrls ?? {}),
        ...Object.keys(effectiveTokenNetworks ?? {}),
        ...Object.keys(effectivePreferredTokens ?? {}),
        ...Object.keys(config.settlementAddresses ?? {}),
      ])
    );

    // Build settlement addresses. Each chain defaults to the identity's EVM
    // address, but a per-chain override (config.settlementAddresses) wins so
    // non-EVM chains can advertise a chain-native recipient (e.g. the apex's
    // base58 Solana pubkey for `solana:devnet`).
    const settlementAddresses: Record<string, string> = {};
    for (const chain of supportedChains) {
      settlementAddresses[chain] =
        config.settlementAddresses?.[chain] ?? identity.evmAddress;
    }

    settlementInfo = {
      supportedChains,
      settlementAddresses,
      preferredTokens: effectivePreferredTokens,
      tokenNetworks: effectiveTokenNetworks,
    };

    if (
      effectiveConnector?.openChannel &&
      effectiveConnector.getChannelState
    ) {
      channelClient = createDirectChannelClient(
        effectiveConnector as Required<
          Pick<EmbeddableConnectorLike, 'openChannel' | 'getChannelState'>
        >
      );
    }
  }

  // --- 7. Connector admin client ---
  // Skipped in oblivious mode (no connector to administer).
  const adminClient: ConnectorAdminClient | undefined = effectiveConnector
    ? createDirectConnectorAdmin(effectiveConnector)
    : undefined;

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
      const result = await registry.dispatch(ctx);
      // Broadcast stored events to WebSocket subscribers so the live event feed
      // in the Townhouse dashboard reflects newly accepted events in real time.
      if (result.accept) {
        try {
          const event = decodeEventFromToon(toonBytes);
          wsRelayRef.current?.broadcastEvent(event);
        } catch {
          // Non-Nostr payloads (e.g. kind:10032 ILP info) may fail decode — ignore.
        }
      }
      return result;
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
  // discoveryTracker is created after the embedded connector starts; this ref
  // lets the health handler respond safely before initialization completes
  // (returns 0 counts until ready).
  const discoveryTrackerRef: {
    current?: ReturnType<typeof createDiscoveryTracker>;
  } = {};

  // wsRelay is created in step 11 (after BLS server). This ref lets handlePacket
  // broadcast newly stored events to WebSocket subscribers without a forward reference.
  const wsRelayRef: { current?: NostrRelayServer } = {};

  // --- 10. BLS HTTP Server ---
  const app = new Hono();
  app.get('/health', (c: Context) => {
    const bootstrapPhase = bootstrapService.getPhase();
    const dt = discoveryTrackerRef.current;
    return c.json(
      createHealthResponse({
        phase: bootstrapPhase,
        pubkey: identity.pubkey,
        ilpAddress,
        peerCount: (dt ? dt.getPeerCount() : 0) + peerCount,
        discoveredPeerCount: dt ? dt.getDiscoveredCount() : 0,
        channelCount,
        basePricePerByte,
        x402Enabled,
        chain: chainConfig.name,
      })
    );
  });

  // The ILP localDelivery write surface is mounted ONLY in embedded mode. In
  // oblivious mode the relay exposes POST /write instead (see below), and
  // /handle-packet is left unmounted (404).
  if (!obliviousMode) {
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
              discoveryTrackerRef.current?.processEvent(decoded);
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
  }

  // --- 10b. ILP client (created before x402 handler so it can be wired in) ---
  // Skipped in oblivious mode (no connector to send ILP packets through).
  const ilpClient: IlpClient | undefined = effectiveConnector
    ? createDirectIlpClient(effectiveConnector, {
        toonDecoder: (bytes: Uint8Array) => decodeEventFromToon(bytes),
      })
    : undefined;

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

  // --- 10d. Write surface routing ---
  // Embedded mode (default): mount the x402 /publish route as before. Oblivious
  // mode: mount POST /write (plain-HTTP, payment-oblivious) and skip /publish.
  if (obliviousMode) {
    // The oblivious write handler stores the event, then mirrors the SAME
    // post-store side effects used by the embedded handlePacket closure:
    //   1. broadcast to WS subscribers so live readers see new events, and
    //   2. feed accepted kind:10032 events to the discovery tracker.
    const obliviousHandler = createObliviousWriteHandler({
      eventStore,
      devMode,
      onStored: (event) => {
        // Mirror handlePacket's WS broadcast (town.ts step 8).
        try {
          wsRelayRef.current?.broadcastEvent(event);
        } catch {
          // Non-broadcastable payloads — ignore (matches embedded behavior).
        }
        // Mirror the discovery-tracker feed for kind:10032 (ILP peer info).
        if (event.kind === ILP_PEER_INFO_KIND) {
          discoveryTrackerRef.current?.processEvent(event);
        }
      },
    });
    app.post('/write', (c: Context) => obliviousHandler.handleWrite(c));
  } else {
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
  }

  const blsServer: ServerType = serve({
    fetch: app.fetch,
    port: blsPort,
  });

  // --- 11. WebSocket Relay ---
  // When ator is enabled, bind to localhost only (hidden service handles inbound routing)
  const relayHost = config.ator?.enabled ? '127.0.0.1' : undefined;
  const wsRelay = new NostrRelayServer(
    { port: relayPort, host: relayHost },
    eventStore
  );
  wsRelayRef.current = wsRelay;
  await wsRelay.start();
  await new Promise((resolve) => setTimeout(resolve, 500));

  // --- 12. Running state ---
  let running = true;

  // --- 13. Bootstrap ---
  // Connector-dependent wiring is skipped in oblivious mode (no connector).
  if (adminClient) {
    bootstrapService.setConnectorAdmin(adminClient);
  }
  if (channelClient) {
    bootstrapService.setChannelClient(channelClient);
  }

  if (ilpClient) {
    bootstrapService.setIlpClient(ilpClient);
  }

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

  // Wire the packet handler directly to the embedded connector.
  if (effectiveConnector?.setPacketHandler) {
    effectiveConnector.setPacketHandler(async (request) => {
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

  // Start the auto-created connector before bootstrap. Pre-built connectors
  // are the caller's responsibility to start.
  if (autoCreatedConnector) {
    await autoCreatedConnector.start();
  }

  // Create DiscoveryTracker
  const discoveryTracker = createDiscoveryTracker({
    secretKey: identity.secretKey,
    settlementInfo,
  });
  if (adminClient) {
    discoveryTracker.setConnectorAdmin(adminClient);
  }
  if (channelClient) {
    discoveryTracker.setChannelClient(channelClient);
  }
  // Wire discovery tracker ref (used by health handler and embedded packet handler)
  discoveryTrackerRef.current = discoveryTracker;

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

  // Handle for the kind:10032 liveness heartbeat (issue #261); cleared in stop().
  let announcementHeartbeat: ReturnType<typeof setInterval> | undefined;

  try {
    const results = await bootstrapService.bootstrap();

    // Self-write: publish own kind:10032
    const ownIlpInfo: IlpPeerInfo = {
      ilpAddress,
      btpEndpoint,
      assetCode,
      assetScale,
      // Advertise the publish price (per byte, in ILP base units) so clients can
      // compute the amount to attach before sending — derived from feePerEvent /
      // basePricePerByte. Previously omitted, leaving peers to assume free.
      feePerByte: String(basePricePerByte),
      // Public Nostr relay URL for FREE reads, so clients discover where to
      // subscribe (separate from btpEndpoint, which is the pay-to-write path).
      // Set when the operator exposes the relay publicly (HS .anyone or direct).
      ...(externalRelayUrl && { relayUrl: externalRelayUrl }),
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

    // Build + store + propagate a fresh kind:10032 each time. Re-signing yields
    // a new created_at and NIP-40 expiration window, so a live apex's
    // announcement stays unexpired while an offline one lapses (issue #261).
    const publishOwnAnnouncement = () => {
      try {
        const ilpInfoEvent = buildIlpPeerInfoEvent(
          ownIlpInfo,
          identity.secretKey,
          announcementTtlSeconds > 0
            ? { ttlSeconds: announcementTtlSeconds }
            : {}
        );
        eventStore.store(ilpInfoEvent);

        // Publish to genesis relay via ILP if we have bootstrap peers
        const firstPeer = knownPeers[0];
        const genesisResult = results[0];
        if (ilpClient && firstPeer && genesisResult) {
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
    };

    publishOwnAnnouncement();

    // Liveness heartbeat: re-announce at half the TTL so there is always an
    // unexpired kind:10032 on the relay while we are up. When the node stops,
    // the heartbeat stops and the last announcement expires after the TTL,
    // signalling clients to stop dialing this (now-unreachable) apex (#261).
    if (announcementTtlSeconds > 0) {
      const heartbeatMs = Math.max(
        1,
        Math.floor((announcementTtlSeconds * 1000) / 2)
      );
      announcementHeartbeat = setInterval(publishOwnAnnouncement, heartbeatMs);
      // Don't let the heartbeat keep the process alive on its own.
      announcementHeartbeat.unref?.();
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
      if (ilpClient && firstPeer && genesisResult) {
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
  const activeSubscriptions = new Set<RelaySubscription>();

  // --- 15. Build RelayInstance ---
  const instance: RelayInstance = {
    isRunning() {
      return running;
    },

    subscribe(subscribeRelayUrl: string, filter: Filter): RelaySubscription {
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

      // Stop the kind:10032 liveness heartbeat so the announcement lapses (#261)
      if (announcementHeartbeat) {
        clearInterval(announcementHeartbeat);
        announcementHeartbeat = undefined;
      }

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

      // Stop auto-created connector
      if (autoCreatedConnector) {
        await autoCreatedConnector.stop();
      }

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

// ---------- Deprecated aliases ----------
// The launcher API was renamed from `startTown`/`Town*` to `startRelay`/`Relay*`
// when @toon-protocol/town was merged into @toon-protocol/relay. The old names
// are retained as aliases so existing callers keep working.

/**
 * @deprecated Use {@link startRelay} instead. Retained for backwards
 * compatibility after the town → relay package merge.
 */
export const startTown = startRelay;

/**
 * @deprecated Use {@link RelayConfig} instead.
 */
export type TownConfig = RelayConfig;

/**
 * @deprecated Use {@link RelayInstance} instead.
 */
export type TownInstance = RelayInstance;

/**
 * @deprecated Use {@link ResolvedRelayConfig} instead.
 */
export type ResolvedTownConfig = ResolvedRelayConfig;

/**
 * @deprecated Use {@link RelaySubscription} instead.
 */
export type TownSubscription = RelaySubscription;
