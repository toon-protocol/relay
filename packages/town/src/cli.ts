#!/usr/bin/env node

/**
 * CLI entrypoint for @crosstown/town.
 *
 * Thin wrapper around startTown() that parses CLI flags and environment
 * variables, then delegates all logic to town.ts.
 *
 * Usage:
 *   npx @crosstown/town --mnemonic "abandon abandon ..." --connector-url "http://localhost:8080"
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
Usage: crosstown-town [options]

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
  --help                   Show this help message

Environment Variables:
  CROSSTOWN_MNEMONIC           Same as --mnemonic
  CROSSTOWN_SECRET_KEY         Same as --secret-key
  CROSSTOWN_RELAY_PORT         Same as --relay-port
  CROSSTOWN_BLS_PORT           Same as --bls-port
  CROSSTOWN_DATA_DIR           Same as --data-dir
  CROSSTOWN_CONNECTOR_URL      Same as --connector-url
  CROSSTOWN_CONNECTOR_ADMIN_URL  Same as --connector-admin-url
  CROSSTOWN_KNOWN_PEERS        Same as --known-peers
  CROSSTOWN_DEV_MODE           Same as --dev-mode (set to "true")
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

  const mnemonic =
    values.mnemonic ?? process.env['CROSSTOWN_MNEMONIC'] ?? undefined;

  const secretKeyHex =
    values['secret-key'] ?? process.env['CROSSTOWN_SECRET_KEY'] ?? undefined;

  let secretKey: Uint8Array | undefined;
  if (secretKeyHex) {
    if (secretKeyHex.length !== 64 || !/^[0-9a-fA-F]{64}$/.test(secretKeyHex)) {
      console.error('Error: --secret-key must be a 64-character hex string');
      process.exit(1);
    }
    secretKey = Uint8Array.from(Buffer.from(secretKeyHex, 'hex'));
  }

  const connectorUrl =
    values['connector-url'] ??
    process.env['CROSSTOWN_CONNECTOR_URL'] ??
    undefined;

  if (!connectorUrl) {
    console.error(
      'Error: --connector-url (or CROSSTOWN_CONNECTOR_URL) is required'
    );
    process.exit(1);
  }

  const connectorAdminUrl =
    values['connector-admin-url'] ??
    process.env['CROSSTOWN_CONNECTOR_ADMIN_URL'] ??
    undefined;

  const relayPortStr =
    values['relay-port'] ?? process.env['CROSSTOWN_RELAY_PORT'] ?? undefined;
  const relayPort = relayPortStr ? parseInt(relayPortStr, 10) : undefined;
  if (
    relayPort !== undefined &&
    (Number.isNaN(relayPort) || relayPort <= 0 || relayPort > 65535)
  ) {
    console.error('Error: --relay-port must be an integer between 1 and 65535');
    process.exit(1);
  }

  const blsPortStr =
    values['bls-port'] ?? process.env['CROSSTOWN_BLS_PORT'] ?? undefined;
  const blsPort = blsPortStr ? parseInt(blsPortStr, 10) : undefined;
  if (
    blsPort !== undefined &&
    (Number.isNaN(blsPort) || blsPort <= 0 || blsPort > 65535)
  ) {
    console.error('Error: --bls-port must be an integer between 1 and 65535');
    process.exit(1);
  }

  const dataDir =
    values['data-dir'] ?? process.env['CROSSTOWN_DATA_DIR'] ?? undefined;

  const devMode =
    values['dev-mode'] ??
    (process.env['CROSSTOWN_DEV_MODE'] === 'true' ? true : undefined);

  const knownPeersJson =
    values['known-peers'] ?? process.env['CROSSTOWN_KNOWN_PEERS'] ?? undefined;

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
      'Error: one of --mnemonic (or CROSSTOWN_MNEMONIC) or --secret-key (or CROSSTOWN_SECRET_KEY) is required'
    );
    process.exit(1);
  }

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
  };

  return config;
}

// ---------- Main ----------

async function main(): Promise<void> {
  const config = parseCli();

  console.log('\n' + '='.repeat(50));
  console.log('Crosstown Town Starting');
  console.log('='.repeat(50) + '\n');

  const instance: TownInstance = await startTown(config);

  console.log('\n' + '='.repeat(50));
  console.log('Crosstown Town Ready');
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
