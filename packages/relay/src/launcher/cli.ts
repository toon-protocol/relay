#!/usr/bin/env node

/**
 * CLI entrypoint for @toon-protocol/relay.
 *
 * Thin wrapper around startRelay() that parses CLI flags and environment
 * variables, then delegates to relay.ts. The relay is a plain read/write app:
 * free NIP-01 WebSocket reads plus an HTTP `POST /write` surface. Payment is
 * enforced upstream by an external terminator, so there are no connector,
 * ILP, chain, or pricing options here.
 *
 * Usage:
 *   relay --secret-key <hex>
 *   NOSTR_SECRET_KEY=<hex> relay
 *
 * Environment variables override defaults; CLI flags override environment variables.
 */

import { parseArgs } from 'node:util';
import { startRelay } from './relay.js';
import type { RelayConfig, RelayInstance } from './relay.js';

// ---------- CLI Parsing ----------

function printHelp(): void {
  console.log(
    `
Usage: relay [options]

Options:
  --mnemonic <words>       BIP-39 mnemonic (12 or 24 words; NIP-06 derivation)
  --secret-key <hex>       32-byte secret key in hex
  --relay-port <port>      WebSocket read port (default: 7100)
  --bls-port <port>        HTTP write/health port (default: 3100)
  --host <host>            WebSocket bind host (default: 0.0.0.0)
  --data-dir <path>        Data directory for the SQLite store (default: ./data)
  --dev-mode               Skip event-signature verification on POST /write
  --help                   Show this help message

Environment Variables:
  TOON_MNEMONIC            Same as --mnemonic
  TOON_SECRET_KEY          Same as --secret-key
  NOSTR_SECRET_KEY         Alias for TOON_SECRET_KEY (identity); TOON_SECRET_KEY wins
  TOON_RELAY_PORT          Same as --relay-port
  TOON_BLS_PORT            Same as --bls-port
  TOON_HOST                Same as --host
  TOON_DATA_DIR            Same as --data-dir
  TOON_DEV_MODE            Same as --dev-mode (set to "true")

Security:
  Prefer TOON_MNEMONIC / TOON_SECRET_KEY / NOSTR_SECRET_KEY environment
  variables over --mnemonic / --secret-key CLI flags. CLI arguments are visible
  to other users on the system via process listings (e.g. ps aux). See CWE-214.
`.trim()
  );
}

function parseCli(): RelayConfig {
  const { values } = parseArgs({
    options: {
      mnemonic: { type: 'string' },
      'secret-key': { type: 'string' },
      'relay-port': { type: 'string' },
      'bls-port': { type: 'string' },
      host: { type: 'string' },
      'data-dir': { type: 'string' },
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

  // Identity secret key. NOSTR_SECRET_KEY is accepted as an alias for
  // TOON_SECRET_KEY so the container honors the same identity env the connector
  // compose uses. TOON_SECRET_KEY wins when both are set.
  const secretKeyHex =
    values['secret-key'] ??
    process.env['TOON_SECRET_KEY'] ??
    process.env['NOSTR_SECRET_KEY'] ??
    undefined;

  let secretKey: Uint8Array | undefined;
  if (secretKeyHex) {
    if (secretKeyHex.length !== 64 || !/^[0-9a-fA-F]{64}$/.test(secretKeyHex)) {
      console.error('Error: --secret-key must be a 64-character hex string');
      process.exit(1);
    }
    secretKey = Uint8Array.from(Buffer.from(secretKeyHex, 'hex'));
  }

  if (!mnemonic && !secretKey) {
    console.error(
      'Error: one of --mnemonic (or TOON_MNEMONIC) or --secret-key ' +
        '(or TOON_SECRET_KEY / NOSTR_SECRET_KEY) is required'
    );
    process.exit(1);
  }
  if (mnemonic && secretKey) {
    console.error(
      'Error: provide either a mnemonic or a secret key, not both'
    );
    process.exit(1);
  }

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

  const host = values.host ?? process.env['TOON_HOST'] ?? undefined;

  const dataDir =
    values['data-dir'] ?? process.env['TOON_DATA_DIR'] ?? undefined;

  const devMode =
    values['dev-mode'] ??
    (process.env['TOON_DEV_MODE'] === 'true' ? true : undefined);

  const config: RelayConfig = {
    ...(mnemonic && { mnemonic }),
    ...(secretKey && { secretKey }),
    ...(relayPort !== undefined && { relayPort }),
    ...(blsPort !== undefined && { blsPort }),
    ...(host && { host }),
    ...(dataDir && { dataDir }),
    ...(devMode !== undefined && { devMode }),
  };

  return config;
}

// ---------- Main ----------

async function main(): Promise<void> {
  const config = parseCli();

  console.log('\n' + '='.repeat(50));
  console.log('TOON Relay Starting');
  console.log('='.repeat(50) + '\n');

  const instance: RelayInstance = await startRelay(config);

  console.log('\n' + '='.repeat(50));
  console.log('TOON Relay Ready');
  console.log('='.repeat(50));
  console.log(`  Pubkey:  ${instance.pubkey}`);
  console.log(`  Reads:   ws://localhost:${instance.config.relayPort}`);
  console.log(`  Writes:  http://localhost:${instance.config.blsPort}/write`);
  console.log(`  Health:  http://localhost:${instance.config.blsPort}/health`);
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
