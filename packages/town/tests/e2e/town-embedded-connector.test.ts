/**
 * E2E Test: Town with Embedded Connector
 *
 * Validates that `startTown()` works correctly with an in-process
 * `ConnectorNode` (embedded mode) — the production deployment path used
 * by Oyster CVM and SDK E2E Docker peers.
 *
 * **Prerequisites:**
 * SDK E2E infrastructure running:
 * ```bash
 * ./scripts/sdk-e2e-infra.sh up
 * ```
 *
 * **What this test verifies:**
 * - startTown() composes with an embedded ConnectorNode (zero-latency mode)
 * - BLS health endpoint reports correct status
 * - Relay accepts WebSocket NIP-01 subscriptions
 * - Self-write kind:10032 event is stored on local relay
 * - Inbound ILP packets are handled via embedded connector's setPacketHandler
 */

import { describe, it, expect, beforeAll, afterAll } from 'vitest';
import WebSocket from 'ws';
import {
  generateSecretKey,
  getPublicKey,
  finalizeEvent,
} from 'nostr-tools/pure';
import { ConnectorNode, createLogger } from '@toon-protocol/connector';
import { startTown, type TownInstance } from '@toon-protocol/town';
import {
  encodeEventToToon,
  decodeEventFromToon,
} from '@toon-protocol/core/toon';

// ---------------------------------------------------------------------------
// Constants (SDK E2E infra — see docker-compose-sdk-e2e.yml)
// ---------------------------------------------------------------------------

const ANVIL_RPC = 'http://localhost:18545';

// Peer 1 (Docker)
const PEER1_BLS_URL = 'http://localhost:19100';

// Contracts (deterministic Anvil deployment)
const TOKEN_ADDRESS = '0x5FbDB2315678afecb367f032d93F642f64180aa3';
const TOKEN_NETWORK_ADDRESS = '0xCafac3dD18aC6c6e92c921884f9E4176737C052c';
const REGISTRY_ADDRESS = '0xe7f1725e7734ce288f8367e1bb143e90bb3f0512';

// Anvil Account #3 (used for connector settlement — unused by Docker infra peers)
const TEST_PRIVATE_KEY =
  '0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6';

// Town identity (test mnemonic — Nostr keypair derived internally by startTown)
const TEST_MNEMONIC =
  'test test test test test test test test test test test junk';

// Ports (no conflict with SDK E2E or genesis)
const TOWN_RELAY_PORT = 7600;
const TOWN_BLS_PORT = 3600;
const CONNECTOR_BTP_PORT = 19920;

const CHAIN_ID = 31337;
const BASE_PRICE_PER_BYTE = 10n;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async function waitForHttp(url: string, timeoutMs = 10000): Promise<boolean> {
  const start = Date.now();
  while (Date.now() - start < timeoutMs) {
    try {
      const resp = await fetch(url, { signal: AbortSignal.timeout(2000) });
      if (resp.ok) return true;
    } catch {
      // retry
    }
    await new Promise((r) => setTimeout(r, 500));
  }
  return false;
}

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
      await new Promise((r) => setTimeout(r, 500));
    }
  }
  return false;
}

// ---------------------------------------------------------------------------
// Test suite
// ---------------------------------------------------------------------------

describe('Town E2E with Embedded Connector', () => {
  let servicesReady = false;
  let townInstance: TownInstance | null = null;
  let connector: ConnectorNode;

  beforeAll(async () => {
    // Phase 1: Health check SDK E2E infra (Anvil + Peer1)
    try {
      const [anvilOk, peer1Ok] = await Promise.all([
        fetch(ANVIL_RPC, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({
            jsonrpc: '2.0',
            method: 'eth_blockNumber',
            params: [],
            id: 1,
          }),
          signal: AbortSignal.timeout(3000),
        }).then((r) => r.ok),
        waitForHttp(`${PEER1_BLS_URL}/health`, 10000),
      ]);

      if (!anvilOk || !peer1Ok) {
        console.warn(
          'SDK E2E infra not ready. Run: ./scripts/sdk-e2e-infra.sh up'
        );
        return;
      }
    } catch (error: unknown) {
      console.warn(
        `SDK E2E infra not available: ${error instanceof Error ? error.message : String(error)}`
      );
      return;
    }

    // Phase 2: Create in-process ConnectorNode
    const connectorLogger = createLogger('town-embedded-test', 'warn');
    connector = new ConnectorNode(
      {
        nodeId: 'town-embedded-test',
        btpServerPort: CONNECTOR_BTP_PORT,
        environment: 'development' as const,
        deploymentMode: 'embedded' as const,
        peers: [],
        routes: [],
        localDelivery: { enabled: false },
        settlementInfra: {
          enabled: true,
          rpcUrl: ANVIL_RPC,
          registryAddress: REGISTRY_ADDRESS,
          tokenAddress: TOKEN_ADDRESS,
          privateKey: TEST_PRIVATE_KEY,
        },
      },
      connectorLogger
    );

    await connector.start();

    // Phase 3: Start town with embedded connector
    townInstance = await startTown({
      mnemonic: TEST_MNEMONIC,
      connector,
      relayPort: TOWN_RELAY_PORT,
      blsPort: TOWN_BLS_PORT,
      chainRpcUrls: {
        [`evm:base:${CHAIN_ID}`]: ANVIL_RPC,
      },
      tokenNetworks: {
        [`evm:base:${CHAIN_ID}`]: TOKEN_NETWORK_ADDRESS,
      },
      preferredTokens: {
        [`evm:base:${CHAIN_ID}`]: TOKEN_ADDRESS,
      },
    });

    servicesReady = true;
  }, 120000);

  afterAll(async () => {
    if (townInstance) {
      try {
        await townInstance.stop();
      } catch {
        // ignore
      }
      townInstance = null;
    }

    if (connector) {
      try {
        await connector.stop();
      } catch {
        // ignore
      }
    }

    await new Promise((r) => setTimeout(r, 500));
  });

  // =========================================================================
  // T1: BLS health
  // =========================================================================

  it('BLS /health reports sdk: true and healthy status', async () => {
    if (!servicesReady) {
      console.log('Skipping: SDK E2E infra not ready');
      return;
    }

    const blsReady = await waitForHttp(
      `http://localhost:${TOWN_BLS_PORT}/health`,
      15000
    );
    expect(blsReady).toBe(true);

    const resp = await fetch(`http://localhost:${TOWN_BLS_PORT}/health`);
    const health = (await resp.json()) as Record<string, unknown>;

    expect(health['status']).toBe('healthy');
    expect(health['sdk']).toBe(true);
  });

  // =========================================================================
  // T2: Relay WebSocket
  // =========================================================================

  it('relay accepts WebSocket NIP-01 subscriptions (EOSE)', async () => {
    if (!servicesReady) {
      console.log('Skipping: SDK E2E infra not ready');
      return;
    }

    const relayReady = await waitForWebSocket(
      `ws://localhost:${TOWN_RELAY_PORT}`,
      15000
    );
    expect(relayReady).toBe(true);

    const ws = new WebSocket(`ws://localhost:${TOWN_RELAY_PORT}`);
    const subId = `eose-test-${Date.now()}`;

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
  });

  // =========================================================================
  // T3: Self-write kind:10032
  // =========================================================================

  it('own kind:10032 self-write event present on local relay', async () => {
    if (!servicesReady) {
      console.log('Skipping: SDK E2E infra not ready');
      return;
    }

    const ownPubkey = townInstance!.pubkey;
    expect(ownPubkey).toBeDefined();
    expect(ownPubkey.length).toBe(64);

    const ws = new WebSocket(`ws://localhost:${TOWN_RELAY_PORT}`);
    const subId = `peer-info-${Date.now()}`;

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

    expect(peerInfoEvent).not.toBeNull();
    expect(peerInfoEvent!['kind']).toBe(10032);
    expect(peerInfoEvent!['pubkey']).toBe(ownPubkey);

    const content = JSON.parse(peerInfoEvent!['content'] as string);
    expect(content.ilpAddress).toBeDefined();
    expect(content.btpEndpoint).toBeDefined();
  });

  // =========================================================================
  // T4: Inbound packet handling via embedded connector
  // =========================================================================

  it('handle-packet accepts valid TOON event and stores in relay', async () => {
    if (!servicesReady) {
      console.log('Skipping: SDK E2E infra not ready');
      return;
    }

    // Create a signed Nostr event
    const nostrSecretKey = generateSecretKey();
    const event = finalizeEvent(
      {
        kind: 1,
        content: `Embedded connector inbound test - ${Date.now()}`,
        tags: [['t', 'embedded-connector-test']],
        created_at: Math.floor(Date.now() / 1000),
      },
      nostrSecretKey
    );

    // Encode as TOON and build handle-packet request
    const toonBytes = encodeEventToToon(event);
    const base64Toon = Buffer.from(toonBytes).toString('base64');
    const amount = String(BigInt(toonBytes.length) * BASE_PRICE_PER_BYTE);

    // POST to the town's /handle-packet endpoint (BLS server)
    // This exercises the same pipeline that the embedded connector's
    // setPacketHandler wires up — verification, pricing, handler dispatch.
    const resp = await fetch(
      `http://localhost:${TOWN_BLS_PORT}/handle-packet`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          amount,
          destination: townInstance!.config.ilpAddress,
          data: base64Toon,
        }),
      }
    );

    const result = (await resp.json()) as Record<string, unknown>;
    expect(result['accept']).toBe(true);

    // Verify event was stored in the local relay
    const ws = new WebSocket(`ws://localhost:${TOWN_RELAY_PORT}`);
    const subId = `stored-${Date.now()}`;

    const storedEvent = await new Promise<Record<string, unknown> | null>(
      (resolve) => {
        const timer = setTimeout(() => {
          ws.close();
          resolve(null);
        }, 10000);

        ws.on('open', () => {
          ws.send(JSON.stringify(['REQ', subId, { ids: [event.id] }]));
        });

        ws.on('message', (data: Buffer) => {
          try {
            const msg = JSON.parse(data.toString());
            if (Array.isArray(msg)) {
              if (msg[0] === 'EVENT' && msg[1] === subId && msg[2]) {
                const tBytes = new TextEncoder().encode(msg[2]);
                const decoded = decodeEventFromToon(tBytes);
                clearTimeout(timer);
                ws.close();
                resolve(decoded as unknown as Record<string, unknown>);
              } else if (msg[0] === 'EOSE' && msg[1] === subId) {
                // Event might not be stored yet, wait a bit
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

    expect(storedEvent).not.toBeNull();
    expect(storedEvent!['id']).toBe(event.id);
    expect(storedEvent!['content']).toBe(event.content);
    expect(storedEvent!['pubkey']).toBe(getPublicKey(nostrSecretKey));
  });
});
