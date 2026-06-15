#!/usr/bin/env node

/**
 * CLI entrypoint for @toon-protocol/town.
 *
 * Thin wrapper around startTown() that parses CLI flags and environment
 * variables, then delegates all logic to town.ts.
 *
 * Usage:
 *   npx @toon-protocol/town --mnemonic "abandon abandon ..." \
 *       --connector-url "ws://apex.example:3001" \
 *       --ilp-address "g.townhouse.alice"
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
  --connector-url <url>    Parent connector BTP URL (e.g. ws://apex:3001).
                           When set, the embedded connector peers with this
                           URL and routes everything outside the local prefix
                           through it. --ilp-address becomes REQUIRED and must
                           fall under the parent's prefix.
  --parent-peer-id <id>    BTP peer id to register the parent under (default: apex)
  --parent-auth-token <t>  Auth token for the parent peer (default: empty / no-auth)
  --ilp-address <addr>     ILP address for this node (default: g.toon.<pubkey>;
                           REQUIRED when --connector-url is set, e.g. g.townhouse.<self>)
  --node-id <id>           Stable nodeId for the embedded connector (default: toon-<pubkey>)
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
  TOON_CONNECTOR_URL      Same as --connector-url (parent BTP URL)
  TOON_PARENT_PEER_ID     Same as --parent-peer-id
  TOON_PARENT_AUTH_TOKEN  Same as --parent-auth-token
  TOON_ILP_ADDRESS        Same as --ilp-address (required with TOON_CONNECTOR_URL)
  TOON_NODE_ID            Same as --node-id
  TOON_KNOWN_PEERS        Same as --known-peers
  TOON_DEV_MODE           Same as --dev-mode (set to "true")
  TOON_X402_ENABLED       Same as --x402-enabled (set to "true")
  TOON_DISCOVERY          Same as --discovery
  TOON_SEED_RELAYS        Same as --seed-relays
  TOON_PUBLISH_SEED_ENTRY Same as --publish-seed-entry (set to "true")
  TOON_EXTERNAL_RELAY_URL Same as --external-relay-url
  TOON_FEE_PER_EVENT      Fee per event in ILP units (overrides basePricePerByte)
  TOON_SETTLEMENT_PRIVATE_KEY  EVM private key (0x-prefixed 32-byte hex) for the
                          embedded connector's ClaimReceiver / chainProviders.
                          Defaults to the identity-derived secp256k1 hex.
  TOON_PARENT_EVM_ADDRESS EVM treasury address advertised to the parent
                          connector as the peer entry's evmAddress (used by the
                          apex's PerPacketClaimService when opening a settlement
                          channel toward this child).

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
      'parent-peer-id': { type: 'string' },
      'parent-auth-token': { type: 'string' },
      'ilp-address': { type: 'string' },
      'node-id': { type: 'string' },
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

  const parentPeerId =
    values['parent-peer-id'] ?? process.env['TOON_PARENT_PEER_ID'] ?? undefined;

  const parentAuthToken =
    values['parent-auth-token'] ??
    process.env['TOON_PARENT_AUTH_TOKEN'] ??
    undefined;

  const ilpAddress =
    values['ilp-address'] ?? process.env['TOON_ILP_ADDRESS'] ?? undefined;

  const nodeId = values['node-id'] ?? process.env['TOON_NODE_ID'] ?? undefined;

  if (connectorUrl && !ilpAddress) {
    console.error(
      'Error: --ilp-address (or TOON_ILP_ADDRESS) is required when ' +
        '--connector-url is set; it must fall under the parent connector prefix ' +
        '(e.g. g.townhouse.<self>)'
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

  // Fee per event (overrides basePricePerByte)
  const feePerEventStr = process.env['TOON_FEE_PER_EVENT'] ?? undefined;
  const feePerEvent = feePerEventStr ? parseInt(feePerEventStr, 10) : undefined;
  if (
    feePerEvent !== undefined &&
    (Number.isNaN(feePerEvent) || feePerEvent < 0)
  ) {
    console.error('Error: TOON_FEE_PER_EVENT must be a non-negative integer');
    process.exit(1);
  }

  // Public BTP endpoint advertised in this town's kind:10032 (so clients learn
  // how to reach the apex to route packets to g.townhouse.town). Set by the
  // Townhouse orchestrator from the apex's .anyone / direct URL.
  const btpEndpoint = process.env['TOON_BTP_ENDPOINT'] ?? undefined;

  // Settlement asset advertised in kind:10032.
  const assetCode = process.env['TOON_ASSET_CODE'] ?? undefined;
  const assetScaleStr = process.env['TOON_ASSET_SCALE'] ?? undefined;
  const assetScale = assetScaleStr ? parseInt(assetScaleStr, 10) : undefined;
  if (
    assetScale !== undefined &&
    (Number.isNaN(assetScale) || assetScale < 0 || assetScale > 18)
  ) {
    console.error('Error: TOON_ASSET_SCALE must be an integer in 0..18');
    process.exit(1);
  }

  // Settlement private key — controls the embedded connector's ClaimReceiver
  // signer. CLI flag intentionally omitted (process listings would expose the
  // key via `ps`, CWE-214). Env-only.
  const settlementPrivateKey =
    process.env['TOON_SETTLEMENT_PRIVATE_KEY'] ?? undefined;
  if (
    settlementPrivateKey !== undefined &&
    !/^0x[0-9a-fA-F]{64}$/.test(settlementPrivateKey)
  ) {
    console.error(
      'Error: TOON_SETTLEMENT_PRIVATE_KEY must be a 0x-prefixed 32-byte hex string'
    );
    process.exit(1);
  }

  // Parent EVM address advertised to the apex peer. Public address — safe to
  // ship via env. Validated as 0x + 40 hex chars (ERC-55 mixed-case allowed).
  const parentEvmAddress = process.env['TOON_PARENT_EVM_ADDRESS'] ?? undefined;
  if (
    parentEvmAddress !== undefined &&
    !/^0x[0-9a-fA-F]{40}$/.test(parentEvmAddress)
  ) {
    console.error(
      'Error: TOON_PARENT_EVM_ADDRESS must be a 0x-prefixed 20-byte hex address'
    );
    process.exit(1);
  }

  // Multi-chain settlement advertisement (additive; opt-in via SUPPORTED_CHAINS).
  //
  // The default single-EVM-chain path (TOON_CHAIN/TOON_RPC_URL) is unchanged.
  // When SUPPORTED_CHAINS is set, parse per-chain env vars so the node can
  // advertise additional chains (notably `solana:devnet`) in kind:10032 with a
  // chain-native settlement recipient. Env key convention mirrors the SDK
  // entrypoint (docker/src/shared.ts): "solana:devnet" -> "SOLANA_DEVNET".
  //   SETTLEMENT_ADDRESS_<KEY>  recipient address advertised for the chain
  //   CHAIN_RPC_URL_<KEY>       RPC URL for the chain
  //   TOKEN_NETWORK_<KEY>       payment-channel program / token-network address
  //   PREFERRED_TOKEN_<KEY>     preferred token (e.g. USDC mint)
  let chainRpcUrls: Record<string, string> | undefined;
  let tokenNetworks: Record<string, string> | undefined;
  let preferredTokens: Record<string, string> | undefined;
  let settlementAddresses: Record<string, string> | undefined;
  const supportedChainsStr = process.env['SUPPORTED_CHAINS'];
  if (supportedChainsStr) {
    const chains = supportedChainsStr
      .split(',')
      .map((s) => s.trim())
      .filter(Boolean);
    for (const chain of chains) {
      const key = chain.replace(/:/g, '_').toUpperCase();
      const addr = process.env[`SETTLEMENT_ADDRESS_${key}`];
      if (addr) (settlementAddresses ??= {})[chain] = addr;
      const rpc = process.env[`CHAIN_RPC_URL_${key}`];
      if (rpc) (chainRpcUrls ??= {})[chain] = rpc;
      const tokenNet = process.env[`TOKEN_NETWORK_${key}`];
      if (tokenNet) (tokenNetworks ??= {})[chain] = tokenNet;
      const token = process.env[`PREFERRED_TOKEN_${key}`];
      if (token) (preferredTokens ??= {})[chain] = token;
      if (!addr) {
        console.warn(
          `[Town] Warning: chain "${chain}" listed in SUPPORTED_CHAINS but no SETTLEMENT_ADDRESS_${key} env var found`
        );
      }
    }
  }

  const config: TownConfig = {
    ...(connectorUrl && { connectorUrl }),
    ...(chainRpcUrls && { chainRpcUrls }),
    ...(tokenNetworks && { tokenNetworks }),
    ...(preferredTokens && { preferredTokens }),
    ...(settlementAddresses && { settlementAddresses }),
    ...(parentPeerId && { parentPeerId }),
    ...(parentAuthToken !== undefined && { parentAuthToken }),
    ...(ilpAddress && { ilpAddress }),
    ...(nodeId && { nodeId }),
    ...(mnemonic && { mnemonic }),
    ...(secretKey && { secretKey }),
    ...(relayPort !== undefined && { relayPort }),
    ...(blsPort !== undefined && { blsPort }),
    ...(dataDir && { dataDir }),
    ...(knownPeers && { knownPeers }),
    ...(devMode !== undefined && { devMode }),
    ...(x402Enabled !== undefined && { x402Enabled }),
    ...(discoveryMode && { discovery: discoveryMode }),
    ...(seedRelaysArr && { seedRelays: seedRelaysArr }),
    ...(publishSeedEntry !== undefined && { publishSeedEntry }),
    ...(externalRelayUrl && { externalRelayUrl }),
    ...(feePerEvent !== undefined && { feePerEvent }),
    ...(btpEndpoint && { btpEndpoint }),
    ...(assetCode && { assetCode }),
    ...(assetScale !== undefined && { assetScale }),
    ...(settlementPrivateKey && { settlementPrivateKey }),
    ...(parentEvmAddress && { parentEvmAddress }),
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
  if (instance.config.connectorUrl) {
    console.log(`  Parent BTP:  ${instance.config.connectorUrl}`);
    console.log(`  Parent Peer: ${instance.config.parentPeerId}`);
  }
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
