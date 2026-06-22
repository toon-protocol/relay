/**
 * Integration tests for oblivious mode (`obliviousMode: true`) on startRelay().
 *
 * Oblivious mode runs the relay as a payment-oblivious app behind an external
 * terminator: no embedded connector is created, no x402/EIP-3009/ILP settlement
 * code runs, and the node exposes a plain-HTTP `POST /write` surface instead of
 * the embedded `/handle-packet` + x402 `/publish` routes. Free NIP-01 WS reads
 * are unchanged.
 *
 * These tests boot a real relay instance and exercise the HTTP + WS surface:
 * - no embedded connector is created (asserted via resolved config)
 * - GET /publish and POST /handle-packet -> 404 (not mounted)
 * - POST /write with a valid signed event -> 200, then the event is queryable
 *   over the NIP-01 WebSocket (proves reads + that the write reached storage
 *   without any payment layer)
 * - x402Enabled resolves false even when passed true alongside obliviousMode
 * - stop() succeeds with no connector to tear down
 */

import { describe, it, expect, afterEach } from 'vitest';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { WebSocket } from 'ws';
import { generateSecretKey, finalizeEvent } from 'nostr-tools/pure';
import type { NostrEvent } from 'nostr-tools/pure';
import { startRelay } from './town.js';
import type { RelayInstance } from './town.js';
import { InMemoryEventStore } from '../storage/index.js';

// High, unlikely-to-collide ports for the test relay/BLS servers.
const RELAY_PORT = 17931;
const BLS_PORT = 13931;

let instance: RelayInstance | undefined;
let dataDir: string | undefined;

afterEach(async () => {
  if (instance) {
    await instance.stop();
    instance = undefined;
  }
  if (dataDir) {
    rmSync(dataDir, { recursive: true, force: true });
    dataDir = undefined;
  }
});

function freshDataDir(): string {
  dataDir = mkdtempSync(join(tmpdir(), 'relay-oblivious-'));
  return dataDir;
}

function createSignedEvent(content: string): {
  event: NostrEvent;
  pubkey: string;
} {
  const sk = generateSecretKey();
  const event = finalizeEvent(
    { kind: 1, content, tags: [], created_at: Math.floor(Date.now() / 1000) },
    sk
  );
  return { event, pubkey: event.pubkey };
}

function waitForOpen(ws: WebSocket): Promise<void> {
  return new Promise((resolve, reject) => {
    if (ws.readyState === WebSocket.OPEN) return resolve();
    ws.once('open', () => resolve());
    ws.once('error', reject);
  });
}

/**
 * Collect WS messages until we see an EOSE for the given subscription id.
 */
function collectUntilEose(
  ws: WebSocket,
  subId: string,
  timeoutMs = 5000
): Promise<unknown[]> {
  return new Promise((resolve, reject) => {
    const messages: unknown[] = [];
    const timer = setTimeout(
      () => reject(new Error('timed out waiting for EOSE')),
      timeoutMs
    );
    ws.on('message', (data: Buffer) => {
      const parsed: unknown = JSON.parse(data.toString());
      messages.push(parsed);
      if (
        Array.isArray(parsed) &&
        parsed[0] === 'EOSE' &&
        parsed[1] === subId
      ) {
        clearTimeout(timer);
        resolve(messages);
      }
    });
  });
}

describe('oblivious mode (startRelay obliviousMode: true)', () => {
  it('starts with NO embedded connector and exposes only /write (404 on /publish + /handle-packet)', async () => {
    const secretKey = generateSecretKey();
    instance = await startRelay({
      secretKey,
      obliviousMode: true,
      chain: 'none',
      relayPort: RELAY_PORT,
      blsPort: BLS_PORT,
      dataDir: freshDataDir(),
      eventStore: new InMemoryEventStore(),
    });

    // Resolved config reflects oblivious mode (no embedded connector).
    expect(instance.config.obliviousMode).toBe(true);
    // Relay-only forces chain to the 'none' sentinel (no settlement provider).
    expect(instance.config.chain).toBe('none');

    // /publish and /handle-packet are NOT mounted in oblivious mode -> 404.
    const publishRes = await fetch(`http://localhost:${BLS_PORT}/publish`);
    expect(publishRes.status).toBe(404);

    const handlePacketRes = await fetch(
      `http://localhost:${BLS_PORT}/handle-packet`,
      {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ amount: '1', destination: 'g.x', data: 'AA==' }),
      }
    );
    expect(handlePacketRes.status).toBe(404);
  });

  it('accepts POST /write and the event becomes queryable over the NIP-01 WebSocket', async () => {
    const secretKey = generateSecretKey();
    instance = await startRelay({
      secretKey,
      obliviousMode: true,
      chain: 'none',
      relayPort: RELAY_PORT,
      blsPort: BLS_PORT,
      dataDir: freshDataDir(),
      eventStore: new InMemoryEventStore(),
    });

    const { event } = createSignedEvent('hello oblivious world');

    const writeRes = await fetch(`http://localhost:${BLS_PORT}/write`, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        // Trusted-but-not-validated payment headers, echoed back.
        'X-TOON-Payer': '0x1111111111111111111111111111111111111111',
        'X-TOON-Amount': '1000',
        'X-TOON-Chain': 'evm:base:8453',
      },
      body: JSON.stringify({ event }),
    });
    expect(writeRes.status).toBe(200);
    const writeBody = (await writeRes.json()) as {
      eventId: string;
      payer?: string;
      amount?: string;
      chain?: string;
    };
    expect(writeBody.eventId).toBe(event.id);
    // Headers are echoed (trusted, not re-validated).
    expect(writeBody.payer).toBe(
      '0x1111111111111111111111111111111111111111'
    );
    expect(writeBody.amount).toBe('1000');
    expect(writeBody.chain).toBe('evm:base:8453');

    // The stored event must be readable over the free NIP-01 WS surface.
    const ws = new WebSocket(`ws://localhost:${RELAY_PORT}`);
    await waitForOpen(ws);
    try {
      const subId = 'oblivioussub';
      const collected = collectUntilEose(ws, subId);
      ws.send(JSON.stringify(['REQ', subId, { kinds: [1] }]));
      const messages = await collected;

      // NIP-01 reads on this relay send events TOON-encoded as a string in
      // EVENT[2] (encodeEventToToonString), not as a JSON object — unchanged
      // by oblivious mode. Assert the encoded payload carries our event id.
      const eventMsg = messages.find(
        (m): m is [string, string, string] =>
          Array.isArray(m) && m[0] === 'EVENT' && m[1] === subId
      );
      expect(eventMsg).toBeDefined();
      expect(typeof (eventMsg as [string, string, string])[2]).toBe('string');
      expect((eventMsg as [string, string, string])[2]).toContain(
        `id: ${event.id}`
      );
    } finally {
      ws.close();
    }
  });

  it('forces x402Enabled to false even when passed true alongside obliviousMode', async () => {
    const secretKey = generateSecretKey();
    instance = await startRelay({
      secretKey,
      obliviousMode: true,
      x402Enabled: true,
      chain: 'none',
      relayPort: RELAY_PORT,
      blsPort: BLS_PORT,
      dataDir: freshDataDir(),
      eventStore: new InMemoryEventStore(),
    });

    expect(instance.config.x402Enabled).toBe(false);

    // /publish must remain unmounted (404) despite x402Enabled: true.
    const publishRes = await fetch(`http://localhost:${BLS_PORT}/publish`);
    expect(publishRes.status).toBe(404);
  });

  it('stop() succeeds with no connector to tear down', async () => {
    const secretKey = generateSecretKey();
    instance = await startRelay({
      secretKey,
      obliviousMode: true,
      chain: 'none',
      relayPort: RELAY_PORT,
      blsPort: BLS_PORT,
      dataDir: freshDataDir(),
      eventStore: new InMemoryEventStore(),
    });
    expect(instance.isRunning()).toBe(true);
    await instance.stop();
    expect(instance.isRunning()).toBe(false);
    instance = undefined; // already stopped; skip afterEach double-stop
  });

  it('rejects obliviousMode combined with connectorUrl', async () => {
    await expect(
      startRelay({
        secretKey: generateSecretKey(),
        obliviousMode: true,
        connectorUrl: 'ws://apex.example:3001',
        ilpAddress: 'g.townhouse.x',
        relayPort: RELAY_PORT,
        blsPort: BLS_PORT,
      })
    ).rejects.toThrow(/mutually exclusive/);
  });
});
