#!/usr/bin/env node

/**
 * CLI entrypoint for @toon-protocol/town.
 *
 * Thin wrapper around startTown() that parses CLI flags and environment
 * variables, then delegates all logic to town.ts.
 *
 * Usage:
 *   npx @toon-protocol/town --mnemonic "abandon abandon ..." --connector-url "http://localhost:8080"
 *
 * Environment variables override defaults; CLI flags override environment variables.
 */

import { parseArgs } from 'node:util';
import { startTown } from './town.js';
import type { TownConfig, TownInstance } from './town.js';

// ---------- CLI Parsing ----------

function printHelp(): void {
  console.log(
    `
Usage: toon-town [options]

Options:
  --mnemonic <words>       BIP-39 mnemonic (12 or 24 words)
  --secret-key <hex>       32-byte secret key in hex
  --relay-port <port>      WebSocket relay port (default: 7100)
  --bls-port <port>        BLS HTTP port (default: 3100)
  --data-dir <path>        Data directory (default: ./data)
  --connector-url <url>    External connector URL (required)
  --connector-admin-url <url>  Connector admin URL (default: connectorUrl port+1)
  --known-peers <json>     Known peers as JSON array
  --dev-mode               Enable dev mode (skip verification)
  --x402-enabled           Enable x402 /publish endpoint (default: false)
  --discovery <mode>       Discovery mode: 'seed-list' or 'genesis' (default: 'genesis')
  --seed-relays <urls>     Comma-separated public Nostr relay URLs for seed discovery
  --publish-seed-entry     Publish this node as a seed relay entry (default: false)
  --external-relay-url <url>  External WebSocket URL of this relay
  --help                   Show this help message

Environment Variables:
  TOON_MNEMONIC           Same as --mnemonic
  TOON_SECRET_KEY         Same as --secret-key
  TOON_RELAY_PORT         Same as --relay-port
  TOON_BLS_PORT           Same as --bls-port
  TOON_DATA_DIR           Same as --data-dir
  TOON_CONNECTOR_URL      Same as --connector-url
  TOON_CONNECTOR_ADMIN_URL  Same as --connector-admin-url
  TOON_KNOWN_PEERS        Same as --known-peers
  TOON_DEV_MODE           Same as --dev-mode (set to "true")
  TOON_X402_ENABLED       Same as --x402-enabled (set to "true")
  TOON_DISCOVERY          Same as --discovery
  TOON_SEED_RELAYS        Same as --seed-relays
  TOON_PUBLISH_SEED_ENTRY Same as --publish-seed-entry (set to "true")
  TOON_EXTERNAL_RELAY_URL Same as --external-relay-url

Security:
  Prefer TOON_MNEMONIC or TOON_SECRET_KEY environment variables
  over --mnemonic / --secret-key CLI flags. CLI arguments are visible to
  other users on the system via process listings (e.g. ps aux). See CWE-214.
`.trim()
  );
}

function parseCli(): TownConfig {
  const { values } = parseArgs({
    options: {
      mnemonic: { type: 'string' },
      'secret-key': { type: 'string' },
      'relay-port': { type: 'string' },
      'bls-port': { type: 'string' },
      'data-dir': { type: 'string' },
      'connector-url': { type: 'string' },
      'connector-admin-url': { type: 'string' },
      'known-peers': { type: 'string' },
      'dev-mode': { type: 'boolean' },
      'x402-enabled': { type: 'boolean' },
      discovery: { type: 'string' },
      'seed-relays': { type: 'string' },
      'publish-seed-entry': { type: 'boolean' },
      'external-relay-url': { type: 'string' },
      help: { type: 'boolean' },
    },
    strict: true,
    allowPositionals: false,
  });

  if (values.help) {
    printHelp();
    process.exit(0);
  }

  // Resolve: CLI flags override env vars

  // Warn about process-listing exposure (CWE-214) when secrets are passed via CLI flags
  if (values.mnemonic) {
    console.warn(
      'Warning: --mnemonic is visible in process listings. ' +
        'Prefer TOON_MNEMONIC environment variable for production use.'
    );
  }
  if (values['secret-key']) {
    console.warn(
      'Warning: --secret-key is visible in process listings. ' +
        'Prefer TOON_SECRET_KEY environment variable for production use.'
    );
  }

  const mnemonic = values.mnemonic ?? process.env['TOON_MNEMONIC'] ?? undefined;

  const secretKeyHex =
    values['secret-key'] ?? process.env['TOON_SECRET_KEY'] ?? undefined;

  let secretKey: Uint8Array | undefined;
  if (secretKeyHex) {
    if (secretKeyHex.length !== 64 || !/^[0-9a-fA-F]{64}$/.test(secretKeyHex)) {
      console.error('Error: --secret-key must be a 64-character hex string');
      process.exit(1);
    }
    secretKey = Uint8Array.from(Buffer.from(secretKeyHex, 'hex'));
  }

  const connectorUrl =
    values['connector-url'] ?? process.env['TOON_CONNECTOR_URL'] ?? undefined;

  if (!connectorUrl) {
    console.error('Error: --connector-url (or TOON_CONNECTOR_URL) is required');
    process.exit(1);
  }

  const connectorAdminUrl =
    values['connector-admin-url'] ??
    process.env['TOON_CONNECTOR_ADMIN_URL'] ??
    undefined;

  const relayPortStr =
    values['relay-port'] ?? process.env['TOON_RELAY_PORT'] ?? undefined;
  const relayPort = relayPortStr ? parseInt(relayPortStr, 10) : undefined;
  if (
    relayPort !== undefined &&
    (Number.isNaN(relayPort) || relayPort <= 0 || relayPort > 65535)
  ) {
    console.error('Error: --relay-port must be an integer between 1 and 65535');
    process.exit(1);
  }

  const blsPortStr =
    values['bls-port'] ?? process.env['TOON_BLS_PORT'] ?? undefined;
  const blsPort = blsPortStr ? parseInt(blsPortStr, 10) : undefined;
  if (
    blsPort !== undefined &&
    (Number.isNaN(blsPort) || blsPort <= 0 || blsPort > 65535)
  ) {
    console.error('Error: --bls-port must be an integer between 1 and 65535');
    process.exit(1);
  }

  const dataDir =
    values['data-dir'] ?? process.env['TOON_DATA_DIR'] ?? undefined;

  const devMode =
    values['dev-mode'] ??
    (process.env['TOON_DEV_MODE'] === 'true' ? true : undefined);

  const x402Enabled =
    values['x402-enabled'] ??
    (process.env['TOON_X402_ENABLED'] === 'true' ? true : undefined);

  const knownPeersJson =
    values['known-peers'] ?? process.env['TOON_KNOWN_PEERS'] ?? undefined;

  let knownPeers:
    | { pubkey: string; relayUrl: string; btpEndpoint: string }[]
    | undefined;
  if (knownPeersJson) {
    try {
      const parsed: unknown = JSON.parse(knownPeersJson);
      if (Array.isArray(parsed)) {
        knownPeers = (parsed as unknown[])
          .filter(
            (p): p is Record<string, unknown> =>
              typeof p === 'object' &&
              p !== null &&
              typeof (p as Record<string, unknown>)['pubkey'] === 'string' &&
              typeof (p as Record<string, unknown>)['btpEndpoint'] === 'string'
          )
          .map((p) => ({
            pubkey: p['pubkey'] as string,
            relayUrl: (p['relayUrl'] as string) || 'ws://localhost:7100',
            btpEndpoint: p['btpEndpoint'] as string,
          }));
      }
    } catch {
      console.error('Error: --known-peers must be valid JSON');
      process.exit(1);
    }
  }

  if (!mnemonic && !secretKey) {
    console.error(
      'Error: one of --mnemonic (or TOON_MNEMONIC) or --secret-key (or TOON_SECRET_KEY) is required'
    );
    process.exit(1);
  }

  // Discovery mode
  const discoveryStr =
    values.discovery ?? process.env['TOON_DISCOVERY'] ?? undefined;
  let discoveryMode: 'seed-list' | 'genesis' | undefined;
  if (discoveryStr) {
    if (discoveryStr !== 'seed-list' && discoveryStr !== 'genesis') {
      console.error('Error: --discovery must be "seed-list" or "genesis"');
      process.exit(1);
    }
    discoveryMode = discoveryStr;
  }

  // Seed relays (comma-separated list of public Nostr relay URLs)
  const seedRelaysStr =
    values['seed-relays'] ?? process.env['TOON_SEED_RELAYS'] ?? undefined;
  const seedRelaysArr = seedRelaysStr
    ? seedRelaysStr
        .split(',')
        .map((s) => s.trim())
        .filter(Boolean)
    : undefined;

  // Validate seed relay URLs have WebSocket scheme (CWE-20)
  if (seedRelaysArr) {
    for (const url of seedRelaysArr) {
      // nosemgrep: javascript.lang.security.detect-insecure-websocket.detect-insecure-websocket -- validation check, not a connection
      if (!url.startsWith('ws://') && !url.startsWith('wss://')) {
        console.error(
          'Error: --seed-relays contains invalid URL -- must use WebSocket scheme (ws or wss)'
        );
        process.exit(1);
      }
    }
  }

  // Publish seed entry flag
  const publishSeedEntry =
    values['publish-seed-entry'] ??
    (process.env['TOON_PUBLISH_SEED_ENTRY'] === 'true' ? true : undefined);

  // External relay URL
  const externalRelayUrl =
    values['external-relay-url'] ??
    process.env['TOON_EXTERNAL_RELAY_URL'] ??
    undefined;

  const config: TownConfig = {
    connectorUrl,
    ...(mnemonic && { mnemonic }),
    ...(secretKey && { secretKey }),
    ...(relayPort !== undefined && { relayPort }),
    ...(blsPort !== undefined && { blsPort }),
    ...(dataDir && { dataDir }),
    ...(connectorAdminUrl && { connectorAdminUrl }),
    ...(knownPeers && { knownPeers }),
    ...(devMode !== undefined && { devMode }),
    ...(x402Enabled !== undefined && { x402Enabled }),
    ...(discoveryMode && { discovery: discoveryMode }),
    ...(seedRelaysArr && { seedRelays: seedRelaysArr }),
    ...(publishSeedEntry !== undefined && { publishSeedEntry }),
    ...(externalRelayUrl && { externalRelayUrl }),
  };

  return config;
}

// ---------- Main ----------

async function main(): Promise<void> {
  const config = parseCli();

  console.log('\n' + '='.repeat(50));
  console.log('TOON Town Starting');
  console.log('='.repeat(50) + '\n');

  const instance: TownInstance = await startTown(config);

  console.log('\n' + '='.repeat(50));
  console.log('TOON Town Ready');
  console.log('='.repeat(50));
  console.log(`  Pubkey:      ${instance.pubkey}`);
  console.log(`  EVM Address: ${instance.evmAddress}`);
  console.log(`  Relay:       ws://localhost:${instance.config.relayPort}`);
  console.log(`  BLS:         http://localhost:${instance.config.blsPort}`);
  console.log(`  ILP Address: ${instance.config.ilpAddress}`);
  console.log(`  Peers:       ${instance.bootstrapResult.peerCount}`);
  console.log(`  Channels:    ${instance.bootstrapResult.channelCount}`);
  console.log('='.repeat(50) + '\n');

  // Wire graceful shutdown
  const shutdown = async (signal: string): Promise<void> => {
    console.log(`\n[Shutdown] Received ${signal}`);
    await instance.stop();
    console.log('[Shutdown] Complete');
    process.exit(0);
  };

  process.on('SIGINT', () => {
    shutdown('SIGINT').catch(console.error);
  });
  process.on('SIGTERM', () => {
    shutdown('SIGTERM').catch(console.error);
  });
}

main().catch((error: unknown) => {
  console.error('[Fatal] Startup error:', error);
  process.exit(1);
});
