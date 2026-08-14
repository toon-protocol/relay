/**
 * startRelay() -- Programmatic API for starting a TOON relay node.
 *
 * The relay is a plain HTTP/WebSocket app. It does NOT speak ILP and contains
 * no payment, connector, settlement, or pricing logic: payment is enforced
 * entirely upstream by an external terminator (see the connector repo). By the
 * time a write reaches this process it is already proven paid, so the relay
 * simply stores the event and serves reads.
 *
 * Three surfaces:
 *
 *   - `POST /write` (TOON_BLS_PORT, default 3100): accepts `{ event }` as JSON.
 *     By the time a request reaches this surface it is already proven paid;
 *     the terminating connector asserts nothing about that payment to this
 *     relay -- no payer, amount, or chain (`toon-protocol/connector` ADR
 *     0036) -- so the handler verifies only the event's own signature for
 *     integrity (paid ephemeral kinds skip schnorr by default and keep the
 *     id check -- relay#85, see `verifyEphemeral`), and stores it.
 *     `GET /health` and `GET /metrics` live on the same port.
 *   - `POST /write-ephemeral` (same port, relay#129): the FREE ephemeral
 *     write lane. Accepts only ephemeral kinds (NIP-16, 20000 <= kind <
 *     30000), always runs FULL schnorr verification (no skip -- this lane
 *     has no payment gate), and never stores. Bounded by a per-key rate
 *     limit and a body-size cap (see `ephemeralRateLimit` /
 *     `ephemeralMaxBodyBytes`). Terminated at the connector by its own
 *     zero-priced route (`deploy/connector.toml`'s `g.toon.relay.ephemeral`).
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
import type { NostrEvent } from 'nostr-tools/pure';
import type { Filter } from 'nostr-tools/filter';
import { verifyImplementation } from '../crypto/index.js';
import {
  createVerifyPool,
  defaultVerifyWorkers,
} from '../crypto/verify-pool.js';
import { createMetricsRegistry } from './metrics.js';
import { SqliteEventStore } from '../storage/index.js';
import type { EventStore } from '../storage/index.js';
import { NostrRelayServer } from '../websocket/index.js';
import { DEFAULT_RELAY_CONFIG } from '../types.js';
import { RelaySubscriber } from '../subscriber/index.js';
import { createWriteHandler } from './handlers/write-handler.js';
import {
  createEphemeralWriteHandler,
  DEFAULT_EPHEMERAL_RATE_LIMIT,
  DEFAULT_EPHEMERAL_MAX_BODY_BYTES,
} from './handlers/write-ephemeral-handler.js';
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
  /**
   * Bind host for the HTTP write/health listener (default: 0.0.0.0).
   *
   * The write port MUST only be reachable via the payment-gating connector
   * (see `verifyEphemeral`): in the canonical compose deploy that is enforced
   * by NOT host-publishing the port (docker `expose:`, never `ports:` --
   * note that docker `ports:` publishes bypass ufw). When the relay runs
   * directly on a host, bind this to a loopback/internal address instead.
   * A non-internal bind while the ephemeral verify skip is active logs a
   * prominent startup warning (never a hard failure -- topologies vary).
   */
  writeHost?: string;
  /**
   * Maximum concurrent WebSocket read connections (default: 4096; env:
   * TOON_MAX_CONNECTIONS). Connections beyond the cap are closed with 1013.
   * Fd-limit-shaped, not memory-shaped -- see RelayServerConfig
   * .maxConnections for the sizing reasoning (relay#90).
   */
  maxConnections?: number;

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
  /**
   * Run FULL schnorr verification on ephemeral kinds (default: false -- the
   * paid-ephemeral verify skip is ON by default, relay#85).
   *
   * The default skip is safe ONLY because `POST /write` is payment-gated by
   * the upstream connector and clients verify every signature themselves;
   * the SHA-256 event-id check always runs. Community operators fronting the
   * write port with anything other than a payment-gating connector should
   * set this to true (env: TOON_VERIFY_EPHEMERAL=true). See
   * `WriteHandlerConfig.verifyEphemeral` for the full invariant.
   */
  verifyEphemeral?: boolean;
  /**
   * Worker-thread verify pool size for persistent-kind signature
   * verification (default: `max(0, os.cpus().length - 1)`; env:
   * TOON_VERIFY_WORKERS). `0` -- automatic on 1-core boxes -- is the inline
   * escape hatch: verification runs synchronously on the event loop as
   * before. Workers keep bursty agent-writer verify load from stalling the
   * loop and jittering ephemeral frame latency (relay#85).
   */
  verifyWorkers?: number;

  // --- Free ephemeral write lane (relay#129) ---

  /**
   * Per-key sliding-window rate limit for `POST /write-ephemeral` (default:
   * 200 requests / 10s; env: TOON_EPHEMERAL_RATE_LIMIT /
   * TOON_EPHEMERAL_RATE_WINDOW_MS). This lane has no payment gate, so this
   * bound (plus `ephemeralMaxBodyBytes`) IS its admission control -- see
   * `EphemeralWriteHandlerConfig` for the full reasoning.
   */
  ephemeralRateLimit?: { maxRequests: number; windowMs: number };
  /**
   * Request-body size cap in bytes for `POST /write-ephemeral` (default:
   * 8192; env: TOON_EPHEMERAL_MAX_BODY_BYTES).
   */
  ephemeralMaxBodyBytes?: number;

  // --- Observability ---

  /**
   * Log one line per accepted `POST /write` (default: false). Per-event
   * console I/O is measurable tail jitter on the write hot path (relay#85),
   * so this is a debug switch, not an access log.
   */
  logWrites?: boolean;
}

/**
 * Resolved configuration with all defaults applied.
 */
export interface ResolvedRelayConfig {
  relayPort: number;
  blsPort: number;
  host: string;
  writeHost: string;
  maxConnections: number;
  dataDir: string;
  devMode: boolean;
  verifyEphemeral: boolean;
  verifyWorkers: number;
  ephemeralRateLimit: { maxRequests: number; windowMs: number };
  ephemeralMaxBodyBytes: number;
  logWrites: boolean;
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

// ---------- Write-port exposure guard (relay#85) ----------

/**
 * Whether `host` is a bind address that cannot be reached from the public
 * internet directly: loopback, RFC1918 private, IPv6 unique-local/link-local.
 * `0.0.0.0` / `::` (all interfaces) and public addresses return false.
 *
 * Used by the startup exposure guard: with the paid-ephemeral verify skip
 * active, the write port must only be reachable via the payment-gating
 * connector. Note a "false" here is not proof of exposure -- inside a
 * container, 0.0.0.0 is required for the connector to dial the compose
 * network and the port is kept private by not host-publishing it -- which is
 * why the guard warns instead of failing.
 *
 * @internal Exported for unit testing.
 */
export function isInternalBindHost(host: string): boolean {
  const h = host.trim().toLowerCase();
  if (h === 'localhost' || h === '::1' || h === '[::1]') return true;
  if (h.startsWith('127.')) return true; // 127.0.0.0/8 loopback
  if (h.startsWith('10.')) return true; // 10.0.0.0/8 RFC1918
  if (h.startsWith('192.168.')) return true; // 192.168.0.0/16 RFC1918
  if (/^172\.(1[6-9]|2\d|3[01])\./.test(h)) return true; // 172.16.0.0/12
  if (/^f[cd][0-9a-f]{2}:/.test(h)) return true; // fc00::/7 unique-local
  if (h.startsWith('fe80:')) return true; // link-local
  return false;
}

/**
 * Log the prominent write-port exposure warning when the paid-ephemeral
 * verify skip is active and the write listener binds a non-internal
 * interface. Deliberately a warning, not a hard failure: in the canonical
 * compose deploy the port binds 0.0.0.0 inside the container and is private
 * because it is never host-published (docker `expose:`, not `ports:`).
 *
 * `POST /write-ephemeral` (relay#129) shares this same port/host but needs
 * no exposure check of its own: it always runs full verification (no skip
 * for exposure to weaken) and enforces its own rate limit/size cap
 * regardless of how a request reaches it. This guard's warning is scoped to
 * the paid-write skip above, which stays the one exposure risk on this port.
 *
 * @internal Exported for unit testing.
 */
export function warnIfWritePortExposed(
  writeHost: string,
  blsPort: number,
  options: { verifyEphemeral: boolean; devMode: boolean }
): boolean {
  const skipActive = options.devMode || !options.verifyEphemeral;
  if (!skipActive || isInternalBindHost(writeHost)) {
    return false;
  }
  console.warn(
    [
      '',
      '!'.repeat(72),
      `[relay] WARNING: POST /write is binding ${writeHost}:${blsPort} (a`,
      '[relay] non-loopback/non-internal interface) while event verification',
      options.devMode
        ? '[relay] is fully DISABLED (devMode).'
        : '[relay] is SKIPPED for paid ephemeral kinds (relay#85 default).',
      '[relay] This is safe ONLY if the write port is reachable exclusively',
      '[relay] through the payment-gating connector. In docker, do NOT',
      '[relay] host-publish this port (`expose:`, never `ports:` -- published',
      '[relay] ports bypass ufw). If the port is directly reachable, either',
      '[relay] bind it internally (TOON_WRITE_HOST=127.0.0.1) or restore full',
      '[relay] verification (TOON_VERIFY_EPHEMERAL=true).',
      '!'.repeat(72),
      '',
    ].join('\n')
  );
  return true;
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
  const writeHost = config.writeHost ?? '0.0.0.0';
  const maxConnections =
    config.maxConnections ?? DEFAULT_RELAY_CONFIG.maxConnections;
  const dataDir = config.dataDir ?? './data';
  const devMode = config.devMode ?? false;
  const verifyEphemeral = config.verifyEphemeral ?? false;
  const verifyWorkers = config.verifyWorkers ?? defaultVerifyWorkers();
  const ephemeralRateLimit =
    config.ephemeralRateLimit ?? DEFAULT_EPHEMERAL_RATE_LIMIT;
  const ephemeralMaxBodyBytes =
    config.ephemeralMaxBodyBytes ?? DEFAULT_EPHEMERAL_MAX_BODY_BYTES;
  const logWrites = config.logWrites ?? false;

  const resolvedConfig: ResolvedRelayConfig = {
    relayPort,
    blsPort,
    host,
    writeHost,
    maxConnections,
    dataDir,
    devMode,
    verifyEphemeral,
    verifyWorkers,
    ephemeralRateLimit,
    ephemeralMaxBodyBytes,
    logWrites,
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
  const wsRelay = new NostrRelayServer(
    { port: relayPort, host, maxConnections },
    eventStore
  );

  // --- 5. HTTP write/health server ---
  const app = new Hono();

  app.get('/health', (c: Context) =>
    c.json(createHealthResponse({ pubkey: identity.pubkey }))
  );

  // Metrics registry + verify pool (relay#85): event-loop lag and per-event
  // verify time are the trigger metrics for scaling decisions; the pool
  // keeps persistent-kind verify bursts off the event loop.
  const metrics = createMetricsRegistry({
    verifyImplementation,
    verifyWorkers: 0, // updated once the pool reports its live size below
    ephemeralRateLimit,
    ephemeralMaxBodyBytes,
  });
  const verifyPool = createVerifyPool({
    size: verifyWorkers,
    onMeasure: (ms) => metrics.recordVerify(ms),
  });
  metrics.setVerifyWorkers(verifyPool.size);

  app.get('/metrics', (c: Context) => c.json(metrics.snapshot()));

  // POST /write: trust the upstream terminator's injected payment headers,
  // verify only the event signature, store, and broadcast to live WS readers.
  // Logged once: the noble fallback is a silent ~7x verify-throughput loss,
  // so make the active implementation visible at startup (relay#85).
  console.log(`[relay] event signature verify: ${verifyImplementation}`);
  console.log(
    `[relay] ephemeral-kind schnorr verify: ${
      devMode
        ? 'skipped (devMode)'
        : verifyEphemeral
          ? 'full (TOON_VERIFY_EPHEMERAL)'
          : 'skipped -- payment-gated write path, id check kept (relay#85)'
    }`
  );
  // Exposure guard (relay#85): the verify skip assumes the write port is only
  // reachable via the payment-gating connector.
  warnIfWritePortExposed(writeHost, blsPort, { verifyEphemeral, devMode });
  console.log(
    `[relay] verify pool: ${
      verifyPool.size > 0
        ? `${verifyPool.size} worker thread(s)`
        : 'inline (0 workers -- verification on the event loop)'
    }`
  );
  // Shared by both write surfaces: the pool-backed verifier and the live-WS
  // broadcast hook behave identically on either lane -- only WHEN each lane
  // calls them differs (the paid lane may skip verify for ephemeral kinds;
  // the free lane never does).
  const verifyViaPool = (event: NostrEvent): Promise<boolean> => {
    // Keep the reported worker count honest if the pool degraded.
    metrics.setVerifyWorkers(verifyPool.size);
    return verifyPool.verify(event);
  };
  const broadcastToReaders = (event: NostrEvent): void => {
    try {
      wsRelay.broadcastEvent(event);
    } catch {
      // Non-broadcastable payloads -- ignore.
    }
  };

  const writeHandler = createWriteHandler({
    eventStore,
    devMode,
    verifyEphemeral,
    verifyEvent: verifyViaPool,
    logWrites,
    onStored: broadcastToReaders,
  });
  app.post('/write', (c: Context) => writeHandler.handleWrite(c));

  // POST /write-ephemeral (relay#129): the FREE ephemeral write lane. No
  // payment gate, so unlike /write above, verification here is ALWAYS full
  // (never skipped) and the rate limit + size cap below ARE its admission
  // control -- see write-ephemeral-handler.ts's module doc for the full
  // reasoning. Shares the verify pool with the paid handler.
  console.log(
    `[relay] ephemeral free write lane: enabled on POST /write-ephemeral ` +
      `(full schnorr verify always; rate limit ` +
      `${ephemeralRateLimit.maxRequests} req / ${ephemeralRateLimit.windowMs}ms per key; ` +
      `max body ${ephemeralMaxBodyBytes} bytes)`
  );
  const ephemeralWriteHandler = createEphemeralWriteHandler({
    rateLimit: ephemeralRateLimit,
    maxBodyBytes: ephemeralMaxBodyBytes,
    verifyEvent: verifyViaPool,
    logWrites,
    onBroadcast: broadcastToReaders,
  });
  app.post('/write-ephemeral', (c: Context) =>
    ephemeralWriteHandler.handleWrite(c)
  );

  // Resolve once the HTTP server is actually listening so callers (and tests)
  // never race a not-yet-bound port.
  const blsServer: ServerType = await new Promise<ServerType>((resolve) => {
    const server = serve(
      { fetch: app.fetch, port: blsPort, hostname: writeHost },
      () => resolve(server)
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
      metrics.stop();
      await verifyPool.destroy();

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
