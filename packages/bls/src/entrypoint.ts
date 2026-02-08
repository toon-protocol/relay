import { serve } from '@hono/node-server';
import { Hono } from 'hono';
import { BusinessLogicServer } from './bls/index.js';
import type { BlsEnvConfig } from './config.js';
import { loadBlsConfigFromEnv } from './config.js';
import { ConfigError } from './errors.js';
import { PricingService } from './pricing/index.js';
import { PricingError } from './pricing/types.js';
import { createEventStore } from './storage/index.js';

/**
 * Standalone Docker entrypoint for the BLS.
 *
 * Environment variables:
 * - NODE_ID: Unique identifier for this node (required)
 * - NOSTR_SECRET_KEY: Hex-encoded Nostr secret key (required)
 * - ILP_ADDRESS: ILP address for this node (required)
 * - BLS_PORT: Port to listen on (default: 3100)
 * - BLS_BASE_PRICE_PER_BYTE / BASE_PRICE_PER_BYTE: Base price per byte (default: 10)
 * - OWNER_PUBKEY: Owner pubkey for self-write bypass (optional)
 * - DATA_DIR: Directory for persistent data (default: /data)
 * - BLS_KIND_OVERRIDES / KIND_OVERRIDES: JSON object mapping kind to price (optional)
 */

async function main(): Promise<void> {
  // Load and validate all configuration
  let config: BlsEnvConfig;
  try {
    config = loadBlsConfigFromEnv();
  } catch (error) {
    if (error instanceof ConfigError || error instanceof PricingError) {
      console.error(`ERROR: ${error.message}`);
    } else {
      console.error(`ERROR: Configuration failed: ${error instanceof Error ? error.message : String(error)}`);
    }
    process.exit(1);
  }

  const {
    nodeId,
    pubkey,
    ilpAddress,
    port,
    basePricePerByte,
    ownerPubkey,
    dataDir,
    kindOverrides,
  } = config;

  // Create event store with automatic fallback
  const { eventStore, storageSummary } = createEventStore(dataDir);

  // Create pricing service
  const pricingService = new PricingService({
    basePricePerByte,
    kindOverrides,
  });

  // Create BLS
  const bls = new BusinessLogicServer(
    {
      basePricePerByte,
      pricingService,
      ownerPubkey,
    },
    eventStore
  );

  // Create enhanced app with additional health info
  const app = new Hono();

  // Register enhanced health endpoint BEFORE mounting sub-app
  // (Hono uses first-match-wins routing)
  app.get('/health', (c) => {
    return c.json({
      status: 'healthy',
      nodeId,
      pubkey,
      ilpAddress,
      timestamp: Date.now(),
    });
  });

  // Mount BLS routes
  app.route('/', bls.getApp());

  // Start server
  const server = serve({
    fetch: app.fetch,
    port,
  });

  // Log complete configuration summary
  console.log(`BLS started on port ${port}`);
  console.log(`  Node ID:            ${nodeId}`);
  console.log(`  Pubkey:             ${pubkey}`);
  console.log(`  ILP Address:        ${ilpAddress}`);
  console.log(`  Data Dir:           ${dataDir}`);
  console.log(`  Storage:            ${storageSummary}`);
  console.log(`  Base Price/Byte:    ${basePricePerByte}`);
  if (ownerPubkey) {
    console.log(`  Owner Pubkey:       ${ownerPubkey}`);
  }
  if (kindOverrides && kindOverrides.size > 0) {
    const overridesStr = Array.from(kindOverrides.entries())
      .map(([kind, price]) => `kind ${kind}=${price}`)
      .join(', ');
    console.log(`  Kind Overrides:     ${overridesStr}`);
  }

  // Graceful shutdown
  const shutdown = () => {
    console.log('Shutting down BLS...');
    server.close();
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
