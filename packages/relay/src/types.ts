/**
 * Configuration options for the Nostr relay.
 */
export interface RelayConfig {
  /** Port to listen on (default: 7000) */
  port: number;
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
export const DEFAULT_RELAY_CONFIG: Required<RelayConfig> = {
  port: 7000,
  maxConnections: 100,
  maxSubscriptionsPerConnection: 20,
  maxFiltersPerSubscription: 10,
  databasePath: ':memory:',
};
