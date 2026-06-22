/**
 * Integration tests for startRelay() — the relay as a plain HTTP/WebSocket app.
 *
 * The relay has no payment, connector, or settlement layer: it exposes a
 * plain-HTTP `POST /write` surface (trusting injected `X-TOON-*` headers) plus
 * free NIP-01 WebSocket reads. These tests boot a real instance and exercise
 * that surface:
 *   - GET /health -> 200 with identity + capabilities
 *   - GET /publish and POST /handle-packet -> 404 (no payment routes exist)
 *   - POST /write with a valid signed event -> 200, then the event is queryable
 *     over the NIP-01 WebSocket (proves reads + that the write reached storage)
 *   - stop() tears the instance down cleanly
 */

import { describe, it, expect, afterEach } from 'vitest';
import { mkdtempSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { WebSocket } from 'ws';
import { generateSecretKey, finalizeEvent } from 'nostr-tools/pure';
import type { NostrEvent } from 'nostr-tools/pure';
import { startRelay } from './relay.js';
import type { RelayInstance } from './relay.js';
import { InMemoryEventStore } from '../storage/index.js';

// High, unlikely-to-collide port base. Each boot() takes a fresh pair so
// consecutive tests never race on a port still releasing from a prior teardown.
let portCursor = 17930;
let RELAY_PORT = portCursor;
let BLS_PORT = portCursor + 1;

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
  dataDir = mkdtempSync(join(tmpdir(), 'relay-'));
  return dataDir;
}

function createSignedEvent(content: string): NostrEvent {
  const sk = generateSecretKey();
  return finalizeEvent(
    { kind: 1, content, tags: [], created_at: Math.floor(Date.now() / 1000) },
    sk
  );
}

function waitForOpen(ws: WebSocket): Promise<void> {
  return new Promise((resolve, reject) => {
    if (ws.readyState === WebSocket.OPEN) return resolve();
    ws.once('open', () => resolve());
    ws.once('error', reject);
  });
}

/** Collect WS messages until we see an EOSE for the given subscription id. */
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
      if (Array.isArray(parsed) && parsed[0] === 'EOSE' && parsed[1] === subId) {
        clearTimeout(timer);
        resolve(messages);
      }
    });
  });
}

async function boot(): Promise<RelayInstance> {
  portCursor += 2;
  RELAY_PORT = portCursor;
  BLS_PORT = portCursor + 1;
  return startRelay({
    secretKey: generateSecretKey(),
    relayPort: RELAY_PORT,
    blsPort: BLS_PORT,
    dataDir: freshDataDir(),
    eventStore: new InMemoryEventStore(),
  });
}

describe('startRelay() — HTTP/WS relay app', () => {
  it('serves /health and exposes NO payment routes (/publish + /handle-packet -> 404)', async () => {
    instance = await boot();

    const health = await fetch(`http://localhost:${BLS_PORT}/health`);
    expect(health.status).toBe(200);
    const healthBody = (await health.json()) as {
      status: string;
      pubkey: string;
      capabilities: string[];
    };
    expect(healthBody.status).toBe('healthy');
    expect(healthBody.pubkey).toBe(instance.pubkey);
    expect(healthBody.capabilities).toEqual(['relay']);

    // No payment surface exists.
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
    instance = await boot();

    const event = createSignedEvent('hello relay world');

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
    expect(writeBody.payer).toBe(
      '0x1111111111111111111111111111111111111111'
    );
    expect(writeBody.amount).toBe('1000');
    expect(writeBody.chain).toBe('evm:base:8453');

    // The stored event must be readable over the free NIP-01 WS surface.
    const ws = new WebSocket(`ws://localhost:${RELAY_PORT}`);
    await waitForOpen(ws);
    try {
      const subId = 'sub1';
      const collected = collectUntilEose(ws, subId);
      ws.send(JSON.stringify(['REQ', subId, { kinds: [1] }]));
      const messages = await collected;

      // NIP-01 reads send events TOON-encoded as a string in EVENT[2]
      // (encodeEventToToonString), not as a JSON object. Assert the encoded
      // payload carries our event id.
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

  it('rejects a write with an invalid event signature (422) and stores nothing', async () => {
    instance = await boot();

    const event = createSignedEvent('tampered');
    const tampered = { ...event, content: 'tampered after signing' };

    const res = await fetch(`http://localhost:${BLS_PORT}/write`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ event: tampered }),
    });
    expect(res.status).toBe(422);
  });

  it('isRunning()/stop() manage lifecycle cleanly', async () => {
    instance = await boot();
    expect(instance.isRunning()).toBe(true);
    await instance.stop();
    expect(instance.isRunning()).toBe(false);
    instance = undefined; // already stopped; skip afterEach double-stop
  });

  it('requires exactly one of mnemonic/secretKey', async () => {
    await expect(startRelay({})).rejects.toThrow(/one of mnemonic or secretKey/);
  });
});
