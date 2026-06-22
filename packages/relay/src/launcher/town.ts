/**
 * startRelay() -- Programmatic API for starting a TOON relay node.
 *
 * The relay is a plain HTTP/WebSocket app. It does NOT speak ILP and contains
 * no payment, connector, settlement, or pricing logic: payment is enforced
 * entirely upstream by an external terminator (see the connector repo). By the
 * time a write reaches this process it is already proven paid, so the relay
 * simply stores the event and serves reads.
 *
 * Two surfaces:
 *
 *   - `POST /write` (TOON_BLS_PORT, default 3100): accepts `{ event }` as JSON,
 *     trusts the injected `X-TOON-Payer`/`-Amount`/`-Chain` headers WITHOUT
 *     re-validating payment, verifies only the event's own signature for
 *     integrity, and stores it. `GET /health` lives on the same port.
 *   - Free NIP-01 WebSocket reads (TOON_RELAY_PORT, default 7100).
 *
 * `startRelay()` returns a `RelayInstance` with an explicit `.stop()` for
 * lifecycle control (the CLI wraps this with process-signal handling).
 *
 * @module
 */

import { mkdirSync } from 'node:fs';
import { join } from 'node:path';
import { serve, type ServerType } from '@hono/node-server';
import { Hono, type Context } from 'hono';
import { getPublicKey } from 'nostr-tools/pure';
import { privateKeyFromSeedWords } from 'nostr-tools/nip06';
import type { Filter } from 'nostr-tools/filter';
import { SqliteEventStore } from '../storage/index.js';
import type { EventStore } from '../storage/index.js';
import { NostrRelayServer } from '../websocket/index.js';
import { RelaySubscriber } from '../subscriber/index.js';
import { createObliviousWriteHandler } from './handlers/oblivious-write-handler.js';
import { createHealthResponse } from './health.js';

// ---------- Configuration ----------

/**
 * Configuration for starting a TOON relay node via `startRelay()`.
 *
 * Exactly one of `mnemonic` or `secretKey` must be provided -- it is the node's
 * Nostr identity (surfaced on `/health`).
 */
export interface RelayConfig {
  // --- Identity (exactly one required) ---

  /** 12-word or 24-word BIP-39 mnemonic phrase (NIP-06 derivation). */
  mnemonic?: string;
  /** 32-byte secp256k1 secret key. */
  secretKey?: Uint8Array;

  // --- Network ---

  /** WebSocket relay (read) port (default: 7100). */
  relayPort?: number;
  /** HTTP write/health port (default: 3100). */
  blsPort?: number;
  /**
   * WebSocket bind host (default: 0.0.0.0). Set to `127.0.0.1` to bind the read
   * port to localhost only (e.g. when an upstream proxy handles inbound).
   */
  host?: string;

  // --- Storage ---

  /** Data directory for the file-backed SQLite store (default: ./data). */
  dataDir?: string;
  /**
   * Pre-built EventStore. When provided, the relay uses it instead of building
   * the default file-backed `SqliteEventStore` under `dataDir` (useful for
   * tests via `InMemoryEventStore`, or to share a store when embedding). The
   * caller owns its lifecycle when supplied.
   */
  eventStore?: EventStore;

  // --- Development ---

  /** Skip event-signature verification on `POST /write` (default: false). */
  devMode?: boolean;
}

/**
 * Resolved configuration with all defaults applied.
 */
export interface ResolvedRelayConfig {
  relayPort: number;
  blsPort: number;
  host: string;
  dataDir: string;
  devMode: boolean;
}

/**
 * A running TOON relay node instance returned by `startRelay()`.
 */
export interface RelayInstance {
  /** Whether the relay is currently running. */
  isRunning(): boolean;

  /** Gracefully stop the relay and release all resources. */
  stop(): Promise<void>;

  /**
   * Subscribe to a remote Nostr relay. Received events are stored in this
   * node's EventStore. Returns a handle for lifecycle management.
   *
   * @param relayUrl - WebSocket URL of the relay to subscribe to.
   * @param filter - Nostr filter (kinds, authors, etc.).
   * @returns A RelaySubscription handle.
   * @throws If the relay is not running.
   */
  subscribe(relayUrl: string, filter: Filter): RelaySubscription;

  /** The node's Nostr x-only public key (64-char hex). */
  pubkey: string;

  /** The resolved configuration with all defaults applied. */
  config: ResolvedRelayConfig;
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

// ---------- Identity ----------

/**
 * Derive the node's Nostr identity from the config. Exactly one of `mnemonic`
 * or `secretKey` must be set.
 *
 * @internal
 */
function deriveIdentity(config: RelayConfig): {
  secretKey: Uint8Array;
  pubkey: string;
} {
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

  const secretKey = hasMnemonic
    ? privateKeyFromSeedWords(config.mnemonic as string)
    : (config.secretKey as Uint8Array);

  return { secretKey, pubkey: getPublicKey(secretKey) };
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
  // non-WebSocket URLs from reaching SimplePool.
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
 * Wires the event store, the HTTP write/health server, and the NIP-01
 * WebSocket read server, then returns a `RelayInstance` for lifecycle control.
 *
 * @param config - Node configuration. One of `mnemonic`/`secretKey` is required.
 * @returns A running RelayInstance.
 * @throws If both or neither of mnemonic/secretKey are provided.
 *
 * @example
 * ```typescript
 * const relay = await startRelay({ secretKey });
 * // ... POST /write on 3100, read NIP-01 on 7100 ...
 * await relay.stop();
 * ```
 */
export async function startRelay(config: RelayConfig): Promise<RelayInstance> {
  // --- 1. Identity ---
  const identity = deriveIdentity(config);

  // --- 2. Resolve config ---
  const relayPort = config.relayPort ?? 7100;
  const blsPort = config.blsPort ?? 3100;
  const host = config.host ?? '0.0.0.0';
  const dataDir = config.dataDir ?? './data';
  const devMode = config.devMode ?? false;

  const resolvedConfig: ResolvedRelayConfig = {
    relayPort,
    blsPort,
    host,
    dataDir,
    devMode,
  };

  // --- 3. Event store ---
  // Use the injected store as-is, or build a file-backed SqliteEventStore.
  let eventStore: EventStore;
  if (config.eventStore) {
    eventStore = config.eventStore;
  } else {
    mkdirSync(dataDir, { recursive: true });
    eventStore = new SqliteEventStore(join(dataDir, 'events.db'));
  }

  // --- 4. WebSocket read server (created first so /write can broadcast) ---
  const wsRelay = new NostrRelayServer({ port: relayPort, host }, eventStore);

  // --- 5. HTTP write/health server ---
  const app = new Hono();

  app.get('/health', (c: Context) =>
    c.json(createHealthResponse({ pubkey: identity.pubkey }))
  );

  // POST /write: trust the upstream terminator's injected payment headers,
  // verify only the event signature, store, and broadcast to live WS readers.
  const writeHandler = createObliviousWriteHandler({
    eventStore,
    devMode,
    onStored: (event) => {
      try {
        wsRelay.broadcastEvent(event);
      } catch {
        // Non-broadcastable payloads -- ignore.
      }
    },
  });
  app.post('/write', (c: Context) => writeHandler.handleWrite(c));

  // Resolve once the HTTP server is actually listening so callers (and tests)
  // never race a not-yet-bound port.
  const blsServer: ServerType = await new Promise<ServerType>((resolve) => {
    const server = serve({ fetch: app.fetch, port: blsPort }, () =>
      resolve(server)
    );
  });

  // --- 6. Start the WS read server ---
  await wsRelay.start();

  // --- 7. Lifecycle ---
  let running = true;
  const activeSubscriptions = new Set<RelaySubscription>();

  const instance: RelayInstance = {
    isRunning() {
      return running;
    },

    subscribe(subscribeRelayUrl: string, filter: Filter): RelaySubscription {
      if (!running) {
        throw new Error('Cannot subscribe: relay is not running');
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

      for (const sub of activeSubscriptions) {
        sub.close();
      }
      activeSubscriptions.clear();

      await wsRelay.stop();
      blsServer.close();

      // Only close a store we created; an injected store is the caller's.
      if (!config.eventStore) {
        eventStore.close?.();
      }
    },

    pubkey: identity.pubkey,
    config: resolvedConfig,
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

/** @deprecated Use {@link RelayConfig} instead. */
export type TownConfig = RelayConfig;

/** @deprecated Use {@link RelayInstance} instead. */
export type TownInstance = RelayInstance;

/** @deprecated Use {@link ResolvedRelayConfig} instead. */
export type ResolvedTownConfig = ResolvedRelayConfig;

/** @deprecated Use {@link RelaySubscription} instead. */
export type TownSubscription = RelaySubscription;
