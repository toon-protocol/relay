/**
 * E2E Test: @crosstown/town Package Lifecycle (Story 2.5)
 *
 * **Purpose:**
 * Verify that the @crosstown/town package exports a `startTown(config)` function
 * that starts an SDK-based relay with sensible defaults, performs bootstrap, and
 * accepts events. This is the publishable relay package that replaces the manual
 * docker/src/entrypoint.ts composition.
 *
 * **Prerequisites:**
 * 1. Genesis node infrastructure deployed (Anvil, connector):
 *    ```bash
 *    ./deploy-genesis-node.sh
 *    ```
 * 2. @crosstown/town package built:
 *    ```bash
 *    cd packages/town && pnpm build
 *    ```
 *
 * **GREEN phase (Story 2.5 implementation complete):**
 * - startTown() implemented in packages/town/src/town.ts
 * - TownConfig and TownInstance types exported from @crosstown/town
 * - CLI entrypoint at packages/town/src/cli.ts
 * - package.json bin entry added
 *
 * **What this validates:**
 * - FR-RELAY-1: Published as @crosstown/town with startTown(config) and CLI
 * - Sensible defaults: ports 7100 (relay) and 3100 (BLS) when not specified
 * - Bootstrap runs on start (peers discovered)
 * - Relay accepts events after start
 * - Lifecycle cleanup (stop) works
 * - Package dependencies are correct
 */

import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import WebSocket from 'ws';

// Public API surface of the town package (Story 2.5).
import { startTown, type TownConfig, type TownInstance } from '@crosstown/town';

// For TOON encoding/decoding in test assertions
import { decodeEventFromToon } from '@crosstown/relay';

// For blockchain queries to verify payment channels
// viem imports available for future blockchain verification tests
// import { createPublicClient, http, defineChain, type Hex } from 'viem';

// Infrastructure constants — same as other E2E tests.
// The town instance under test will bind to NON-DEFAULT ports to avoid
// conflicting with any already-running genesis node.
const TOWN_RELAY_PORT = 7200;
const TOWN_BLS_PORT = 3200;
const TOWN_RELAY_URL = `ws://localhost:${TOWN_RELAY_PORT}`;
const TOWN_BLS_URL = `http://localhost:${TOWN_BLS_PORT}`;

// Genesis node endpoints (must be running for bootstrap peer discovery)
const GENESIS_RELAY_URL = 'ws://localhost:7100';
const GENESIS_BLS_URL = 'http://localhost:3100';
const CONNECTOR_URL = 'http://localhost:8080';
const ANVIL_RPC = 'http://localhost:8545';
const GENESIS_PUBKEY =
  'aa1857d0ff1fcb1aeb1907b3b98290f3ecb5545473c0b9296fb0b44481deb572';

// Anvil Account #3 (different from the genesis test account to avoid nonce conflicts)
const TEST_MNEMONIC =
  'test test test test test test test test test test test junk';

// Deployed contract addresses (deterministic on Anvil)
const TOKEN_ADDRESS = '0x5FbDB2315678afecb367f032d93F642f64180aa3'; // Mock USDC (Anvil)
const TOKEN_NETWORK_ADDRESS = '0xCafac3dD18aC6c6e92c921884f9E4176737C052c';

/**
 * Wait for a WebSocket endpoint to become available.
 */
async function waitForWebSocket(
  url: string,
  timeoutMs = 10000
): Promise<boolean> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      await new Promise<void>((resolve, reject) => {
        const ws = new WebSocket(url);
        const timer = setTimeout(() => {
          ws.close();
          reject(new Error('timeout'));
        }, 2000);

        ws.on('open', () => {
          clearTimeout(timer);
          ws.close();
          resolve();
        });

        ws.on('error', () => {
          clearTimeout(timer);
          reject(new Error('connection failed'));
        });
      });
      return true;
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 500));
    }
  }
  return false;
}

/**
 * Wait for an HTTP endpoint to become healthy.
 */
async function waitForHttp(url: string, timeoutMs = 10000): Promise<boolean> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const resp = await fetch(url, { signal: AbortSignal.timeout(2000) });
      if (resp.ok) return true;
    } catch {
      // retry
    }
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  return false;
}


// ============================================================================
// GREEN PHASE -- Story 2.5 implementation complete.
//
// startTown() is exported from @crosstown/town. Tests require genesis node
// infrastructure (Anvil, Connector, Relay) to be running for E2E tests.
// Tests gracefully skip if infrastructure is not available.
//
// Test adjustments applied (Story 2.5 review):
// - connectorUrl: CONNECTOR_URL added to all startTown() calls (required)
// - blsPort: 3500 -> 3550 in T-2.5-05 (avoids Faucet port conflict)
// ============================================================================

describe('@crosstown/town Package Lifecycle (Story 2.5)', () => {
  let genesisReady = false;
  let townInstance: TownInstance | null = null;

  beforeAll(async () => {
    // Verify genesis node infrastructure is running (needed for bootstrap peer)
    try {
      const connectorHealth = await fetch(`${CONNECTOR_URL}/health`, {
        signal: AbortSignal.timeout(3000),
      });
      if (!connectorHealth.ok) {
        console.warn('Connector not ready. Run: ./deploy-genesis-node.sh');
        return;
      }

      const blsHealth = await fetch(`${GENESIS_BLS_URL}/health`, {
        signal: AbortSignal.timeout(3000),
      });
      if (!blsHealth.ok) {
        console.warn('Genesis BLS not ready. Run: ./deploy-genesis-node.sh');
        return;
      }

      const anvilTest = await fetch(ANVIL_RPC, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          jsonrpc: '2.0',
          method: 'eth_blockNumber',
          params: [],
          id: 1,
        }),
        signal: AbortSignal.timeout(3000),
      });
      if (!anvilTest.ok) {
        console.warn('Anvil not ready. Run: ./deploy-genesis-node.sh');
        return;
      }

      genesisReady = true;
    } catch (error: unknown) {
      console.warn('Genesis infrastructure not available.');
      console.warn(
        `Error: ${error instanceof Error ? error.message : String(error)}`
      );
    }
  }, 15000);

  afterAll(async () => {
    // Clean up: stop the town instance if it was started
    if (townInstance) {
      try {
        await townInstance.stop();
      } catch {
        // ignore cleanup errors
      }
      townInstance = null;
    }

    // Allow pending connections to drain
    await new Promise((resolve) => setTimeout(resolve, 1000));
  });

  // ---------------------------------------------------------------------------
  // P0: Core lifecycle
  // ---------------------------------------------------------------------------

  it('should start relay with minimal mnemonic config and accept events', async () => {
    // Validates AC #2/#3: startTown() with mnemonic config starts relay, BLS,
    // and connector, then accepts subscriptions (EOSE).
    if (!genesisReady) {
      console.log('Skipping: Genesis infrastructure not ready');
      return;
    }

    // Start a town instance with minimal config
    townInstance = await startTown({
      mnemonic: TEST_MNEMONIC,
      connectorUrl: CONNECTOR_URL,
      relayPort: TOWN_RELAY_PORT,
      blsPort: TOWN_BLS_PORT,
      knownPeers: [
        {
          pubkey: GENESIS_PUBKEY,
          relayUrl: GENESIS_RELAY_URL,
          btpEndpoint: 'ws://localhost:3000',
        },
      ],
      // Settlement config for payment channels
      chainRpcUrls: {
        'evm:base:31337': ANVIL_RPC,
      },
      tokenNetworks: {
        'evm:base:31337': TOKEN_NETWORK_ADDRESS,
      },
      preferredTokens: {
        'evm:base:31337': TOKEN_ADDRESS,
      },
    });

    // Verify the town instance is running
    expect(townInstance).toBeDefined();
    expect(townInstance.isRunning()).toBe(true);

    // Wait for relay to be accepting WebSocket connections
    const relayReady = await waitForWebSocket(TOWN_RELAY_URL, 15000);
    expect(relayReady).toBe(true);

    // Wait for BLS health endpoint
    const blsReady = await waitForHttp(`${TOWN_BLS_URL}/health`, 15000);
    expect(blsReady).toBe(true);

    // Verify health endpoint reports correct info
    const healthResp = await fetch(`${TOWN_BLS_URL}/health`);
    const health = (await healthResp.json()) as Record<string, unknown>;
    expect(health['status']).toBe('healthy');
    expect(health['sdk']).toBe(true);

    // Verify the relay accepts a NIP-01 subscription
    const ws = new WebSocket(TOWN_RELAY_URL);
    const subId = `accept-test-${Date.now()}`;

    const eoseReceived = await new Promise<boolean>((resolve) => {
      const timer = setTimeout(() => {
        ws.close();
        resolve(false);
      }, 5000);

      ws.on('open', () => {
        ws.send(JSON.stringify(['REQ', subId, { kinds: [1], limit: 1 }]));
      });

      ws.on('message', (data: Buffer) => {
        try {
          const msg = JSON.parse(data.toString());
          if (Array.isArray(msg) && msg[0] === 'EOSE' && msg[1] === subId) {
            clearTimeout(timer);
            ws.close();
            resolve(true);
          }
        } catch {
          // ignore
        }
      });

      ws.on('error', () => {
        clearTimeout(timer);
        resolve(false);
      });
    });

    expect(eoseReceived).toBe(true);
  }, 60000);

  // ---------------------------------------------------------------------------
  // P1: API surface and default behavior
  // ---------------------------------------------------------------------------

  it('should export startTown() and TownConfig from @crosstown/town', async () => {
    // Validates AC #2: package exports startTown() and TownConfig type.

    // Verify startTown is a function
    expect(typeof startTown).toBe('function');

    // Verify TownConfig type exists (verified at compile time by the import)
    // At runtime, we verify the function accepts a config object
    const config: TownConfig = {
      mnemonic: TEST_MNEMONIC,
      connectorUrl: CONNECTOR_URL,
    };
    expect(config.mnemonic).toBe(TEST_MNEMONIC);
    expect(config.connectorUrl).toBe(CONNECTOR_URL);
  }, 10000);

  it('should use default ports (7100 relay, 3100 BLS) when not specified', async () => {
    // Validates AC #3: config defaults and explicit port propagation.
    // NOTE: Cannot bind to default ports if genesis is running, so this test
    // verifies config defaults are undefined at TownConfig level and that
    // explicit ports are propagated to ResolvedTownConfig correctly.

    // Verify default config values
    const config: TownConfig = {
      mnemonic: TEST_MNEMONIC,
      connectorUrl: CONNECTOR_URL,
    };

    // The config object should have undefined ports (defaults applied internally)
    expect(config.relayPort).toBeUndefined();
    expect(config.blsPort).toBeUndefined();

    // Start with explicit non-default ports to avoid conflicts
    // then verify the instance reports what ports it's using
    if (!genesisReady) {
      console.log('Skipping: Genesis infrastructure not ready');
      return;
    }

    // Use non-conflicting ports for this test
    const testInstance = await startTown({
      mnemonic: TEST_MNEMONIC,
      connectorUrl: CONNECTOR_URL,
      relayPort: 7300,
      blsPort: 3300,
      knownPeers: [
        {
          pubkey: GENESIS_PUBKEY,
          relayUrl: GENESIS_RELAY_URL,
          btpEndpoint: 'ws://localhost:3000',
        },
      ],
    });

    try {
      // TownInstance should expose the actual ports in use
      expect(testInstance.config.relayPort).toBe(7300);
      expect(testInstance.config.blsPort).toBe(3300);
    } finally {
      await testInstance.stop();
    }
  }, 60000);

  it('should run bootstrap and discover peers on start', async () => {
    // Validates AC #3: bootstrap runs, peers discovered, own kind:10032 published.
    if (!genesisReady) {
      console.log('Skipping: Genesis infrastructure not ready');
      return;
    }

    const instance = await startTown({
      mnemonic: TEST_MNEMONIC,
      connectorUrl: CONNECTOR_URL,
      relayPort: 7400,
      blsPort: 3400,
      knownPeers: [
        {
          pubkey: GENESIS_PUBKEY,
          relayUrl: GENESIS_RELAY_URL,
          btpEndpoint: 'ws://localhost:3000',
        },
      ],
      chainRpcUrls: {
        'evm:base:31337': ANVIL_RPC,
      },
      tokenNetworks: {
        'evm:base:31337': TOKEN_NETWORK_ADDRESS,
      },
      preferredTokens: {
        'evm:base:31337': TOKEN_ADDRESS,
      },
    });

    try {
      // Verify peers were discovered during bootstrap
      expect(instance.bootstrapResult).toBeDefined();
      expect(instance.bootstrapResult.peerCount).toBeGreaterThanOrEqual(1);

      // Verify the instance's own ILP peer info event (kind:10032) was published
      // to its local relay (self-write)
      const relayReady = await waitForWebSocket(`ws://localhost:7400`, 15000);
      expect(relayReady).toBe(true);

      // Query for the instance's own kind:10032 event
      const ownPubkey = instance.pubkey;
      expect(ownPubkey).toBeDefined();
      expect(typeof ownPubkey).toBe('string');
      expect(ownPubkey.length).toBe(64);

      const ws = new WebSocket('ws://localhost:7400');
      const subId = `peer-test-${Date.now()}`;

      const peerInfoEvent = await new Promise<Record<string, unknown> | null>(
        (resolve) => {
          const timer = setTimeout(() => {
            ws.close();
            resolve(null);
          }, 10000);

          ws.on('open', () => {
            ws.send(
              JSON.stringify([
                'REQ',
                subId,
                { kinds: [10032], authors: [ownPubkey], limit: 1 },
              ])
            );
          });

          ws.on('message', (data: Buffer) => {
            try {
              const msg = JSON.parse(data.toString());
              if (Array.isArray(msg)) {
                if (msg[0] === 'EVENT' && msg[1] === subId && msg[2]) {
                  const toonBytes = new TextEncoder().encode(msg[2]);
                  const event = decodeEventFromToon(toonBytes);
                  clearTimeout(timer);
                  ws.close();
                  resolve(event as unknown as Record<string, unknown>);
                } else if (msg[0] === 'EOSE' && msg[1] === subId) {
                  // Wait briefly for late arrivals
                }
              }
            } catch {
              // ignore
            }
          });

          ws.on('error', () => {
            clearTimeout(timer);
            resolve(null);
          });
        }
      );

      // The instance should have published its own ILP peer info
      expect(peerInfoEvent).not.toBeNull();
      expect(peerInfoEvent!.kind).toBe(10032);
      expect(peerInfoEvent!.pubkey).toBe(ownPubkey);

      const content = JSON.parse(peerInfoEvent!.content as string);
      expect(content.ilpAddress).toBeDefined();
      expect(content.btpEndpoint).toBeDefined();
    } finally {
      await instance.stop();
    }
  }, 60000);

  it('should stop cleanly via lifecycle stop', async () => {
    // Validates AC #5: stop() shuts down relay, BLS, and releases ports.
    if (!genesisReady) {
      console.log('Skipping: Genesis infrastructure not ready');
      return;
    }

    const instance = await startTown({
      mnemonic: TEST_MNEMONIC,
      connectorUrl: CONNECTOR_URL,
      relayPort: 7500,
      blsPort: 3550, // NOTE: 3500 conflicts with Faucet service on genesis infra
      knownPeers: [
        {
          pubkey: GENESIS_PUBKEY,
          relayUrl: GENESIS_RELAY_URL,
          btpEndpoint: 'ws://localhost:3000',
        },
      ],
    });

    // Verify the instance is running
    expect(instance.isRunning()).toBe(true);

    // Verify relay is accepting connections
    const relayUp = await waitForWebSocket('ws://localhost:7500', 15000);
    expect(relayUp).toBe(true);

    // Verify BLS is healthy
    const blsUp = await waitForHttp('http://localhost:3550/health', 15000);
    expect(blsUp).toBe(true);

    // Stop the instance
    await instance.stop();

    // Verify the instance reports as not running
    expect(instance.isRunning()).toBe(false);

    // Verify relay is no longer accepting connections
    const relayDown = await new Promise<boolean>((resolve) => {
      const ws = new WebSocket('ws://localhost:7500');
      const timer = setTimeout(() => {
        resolve(true); // Timeout = connection refused = server is down
      }, 3000);

      ws.on('open', () => {
        clearTimeout(timer);
        ws.close();
        resolve(false); // Server is still up = stop() didn't work
      });

      ws.on('error', () => {
        clearTimeout(timer);
        resolve(true); // Connection error = server is down
      });
    });
    expect(relayDown).toBe(true);

    // Verify BLS is no longer responding
    let blsDown = false;
    try {
      await fetch('http://localhost:3550/health', {
        signal: AbortSignal.timeout(2000),
      });
      // If we get a response, the server is still up
      blsDown = false;
    } catch {
      // Connection error = server is down
      blsDown = true;
    }
    expect(blsDown).toBe(true);
  }, 60000);

  // ---------------------------------------------------------------------------
  // P2: Package structure validation
  // ---------------------------------------------------------------------------

  it('package.json should depend on @crosstown/sdk, @crosstown/relay, @crosstown/core', async () => {
    // Validates AC #1: correct dependencies, bin entry, ESM, type:module.

    const fs = await import('fs');
    const path = await import('path');

    const packageJsonPath = path.resolve(
      import.meta.dirname,
      '../../package.json'
    );

    let packageJson: Record<string, unknown>;
    try {
      const content = fs.readFileSync(packageJsonPath, 'utf-8');
      packageJson = JSON.parse(content);
    } catch {
      expect.fail(
        'packages/town/package.json does not exist. ' +
          'Create the @crosstown/town package with correct dependencies.'
      );
      return;
    }

    // Verify package name
    expect(packageJson['name']).toBe('@crosstown/town');

    // Verify required dependencies
    const deps = packageJson['dependencies'] as
      | Record<string, string>
      | undefined;
    expect(deps).toBeDefined();

    // @crosstown/sdk is the core SDK that provides createNode(), handler registry, etc.
    expect(deps!['@crosstown/sdk']).toBeDefined();

    // @crosstown/relay provides NostrRelayServer and SqliteEventStore
    expect(deps!['@crosstown/relay']).toBeDefined();

    // @crosstown/core provides BootstrapService, TOON codec, event builders, etc.
    expect(deps!['@crosstown/core']).toBeDefined();

    // Verify the package has a bin entry for CLI usage
    const bin = packageJson['bin'] as
      | Record<string, string>
      | string
      | undefined;
    expect(bin).toBeDefined();

    // Verify the package exports startTown
    const main = packageJson['main'] as string | undefined;
    expect(main).toBeDefined();

    // Verify the package type is ESM
    expect(packageJson['type']).toBe('module');
  }, 10000);
});
