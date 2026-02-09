import { serve, type ServerType } from '@hono/node-server';
import type { Hono } from 'hono';
import { BusinessLogicServer } from './bls/index.js';
import type { EventStore } from './storage/index.js';
import { InMemoryEventStore } from './storage/index.js';
import { SqliteEventStore } from './storage/index.js';
import { PricingService } from './pricing/index.js';

/**
 * Configuration for creating a standalone BLS server.
 */
export interface CreateBlsServerConfig {
  /** Port to listen on (used by start()) */
  port?: number;
  /** Base price per byte for event storage */
  basePricePerByte: bigint;
  /** Optional owner pubkey - events from this pubkey bypass payment */
  ownerPubkey?: string;
  /** Path to SQLite database file. If not provided, uses in-memory store */
  dbPath?: string;
  /** Optional kind-specific price overrides */
  kindOverrides?: Map<number, bigint>;
  /** Optional minimum price for SPSP request events (kind:23194) */
  spspMinPrice?: bigint;
}

/**
 * Result of creating a BLS server.
 */
export interface BlsServerInstance {
  /** The Hono app instance */
  app: Hono;
  /** Start the server on the specified port */
  start(port: number): void;
  /** Stop the server and close resources */
  stop(): void;
}

/**
 * Create a standalone BLS server instance.
 *
 * @param config - Server configuration
 * @returns BlsServerInstance with app, start, and stop methods
 */
export function createBlsServer(config: CreateBlsServerConfig): BlsServerInstance {
  // Create event store
  const eventStore: EventStore = config.dbPath
    ? new SqliteEventStore(config.dbPath)
    : new InMemoryEventStore();

  // Create pricing service
  const pricingService = new PricingService({
    basePricePerByte: config.basePricePerByte,
    kindOverrides: config.kindOverrides,
  });

  // Create BLS
  const bls = new BusinessLogicServer(
    {
      basePricePerByte: config.basePricePerByte,
      pricingService,
      ownerPubkey: config.ownerPubkey,
      spspMinPrice: config.spspMinPrice,
    },
    eventStore
  );

  const app = bls.getApp();
  let server: ServerType | undefined;

  return {
    app,
    start(port: number): void {
      server = serve({
        fetch: app.fetch,
        port,
      });
    },
    stop(): void {
      if (server) {
        server.close();
        server = undefined;
      }
      if (eventStore.close) {
        eventStore.close();
      }
    },
  };
}
