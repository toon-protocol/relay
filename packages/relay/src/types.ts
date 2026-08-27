/**
 * Configuration options for the Nostr relay.
 */
export interface RelayServerConfig {
  /** Port to listen on (default: 7100) */
  port: number;
  /** Host/IP to bind to (default: '0.0.0.0'). Set to '127.0.0.1' for hidden service mode. */
  host?: string;
  /**
   * Maximum concurrent WebSocket connections (default: 4096; relay#90).
   *
   * Each connection costs one file descriptor plus a few KB of handler
   * state, so the practical ceiling is fd-limit-shaped, not memory-shaped.
   * 4096 supports several hundred-listener huddles at once (the stock 100
   * made >100 listeners impossible) while leaving comfortable headroom
   * under docker's default nofile limit (1048576) AND still fitting under a
   * conservative 8192 ulimit; on a classic 1024 soft limit the startup
   * fd-limit check logs a warning (see NostrRelayServer.start).
   */
  maxConnections?: number;
  /** Maximum subscriptions per connection (default: 20) */
  maxSubscriptionsPerConnection?: number;
  /** Maximum filters per subscription (default: 10) */
  maxFiltersPerSubscription?: number;
  /** Path to SQLite database file (default: ':memory:' for in-memory) */
  databasePath?: string;
  /**
   * Enforce NIP-40 expiration on the live broadcast path (default: true).
   *
   * The stored-history path is enforced by the EventStore; this flag covers
   * the other way an event reaches a subscriber — the fan-out of a freshly
   * written event. Both are driven from the same launcher setting so a relay
   * cannot serve an expired event on one path while hiding it on the other.
   */
  enforceExpiration?: boolean;
}

/**
 * Default relay configuration values.
 */
export const DEFAULT_RELAY_CONFIG: Required<RelayServerConfig> = {
  port: 7100,
  host: '0.0.0.0',
  maxConnections: 4096,
  maxSubscriptionsPerConnection: 20,
  maxFiltersPerSubscription: 10,
  databasePath: ':memory:',
  enforceExpiration: true,
};
