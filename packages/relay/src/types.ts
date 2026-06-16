/**
 * Configuration options for the Nostr relay.
 */
export interface RelayServerConfig {
  /** Port to listen on (default: 7000) */
  port: number;
  /** Host/IP to bind to (default: '0.0.0.0'). Set to '127.0.0.1' for hidden service mode. */
  host?: string;
  /** Maximum concurrent connections (default: 100) */
  maxConnections?: number;
  /** Maximum subscriptions per connection (default: 20) */
  maxSubscriptionsPerConnection?: number;
  /** Maximum filters per subscription (default: 10) */
  maxFiltersPerSubscription?: number;
  /** Path to SQLite database file (default: ':memory:' for in-memory) */
  databasePath?: string;
}

/**
 * Default relay configuration values.
 */
export const DEFAULT_RELAY_CONFIG: Required<RelayServerConfig> = {
  port: 7000,
  host: '0.0.0.0',
  maxConnections: 100,
  maxSubscriptionsPerConnection: 20,
  maxFiltersPerSubscription: 10,
  databasePath: ':memory:',
};
