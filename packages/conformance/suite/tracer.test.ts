import { finalizeEvent, generateSecretKey } from 'nostr-tools/pure';
import type { Event } from 'nostr-tools';
import { afterAll, beforeAll, describe, expect } from 'vitest';
import WebSocket from 'ws';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import {
  STUB_ILP_ADDRESS,
  STUB_PRICE,
  STUB_SEAL_KEY,
  STUB_WRITE_EDGE,
} from './harness/stub-connector.js';
import { conformanceTest, imageUnderTest } from './implementation.js';

let relay: RunningRelay;

beforeAll(async () => {
  relay = await startRelay(imageUnderTest());
});

afterAll(async () => {
  await relay?.stop();
});

/** Send a REQ and collect events until EOSE. */
function query(url: string, filter: Record<string, unknown>): Promise<Event[]> {
  return new Promise((resolve, reject) => {
    const ws = new WebSocket(url);
    const events: Event[] = [];
    const timer = setTimeout(() => {
      ws.close();
      reject(new Error('no EOSE'));
    }, 10_000);
    ws.on('error', reject);
    ws.on('open', () => ws.send(JSON.stringify(['REQ', 'conf', filter])));
    ws.on('message', (data) => {
      const message = JSON.parse(String(data)) as [string, ...unknown[]];
      if (message[0] === 'EVENT') events.push(message[2] as Event);
      if (message[0] === 'EOSE') {
        clearTimeout(timer);
        ws.close();
        resolve(events);
      }
    });
  });
}

describe('relay image conformance: tracer', () => {
  conformanceTest('GET /health returns the documented body shape', async () => {
    const response = await fetch(`${relay.writeUrl}/health`);
    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      status: 'healthy',
      pubkey: expect.stringMatching(/^[0-9a-f]{64}$/),
      capabilities: expect.arrayContaining(['relay']),
      version: expect.any(String),
      timestamp: expect.any(Number),
    });
  });

  conformanceTest(
    'a paid write returns 200 with the event id and is served on a REQ then EOSE',
    async () => {
      const event = finalizeEvent(
        {
          kind: 1,
          created_at: Math.floor(Date.now() / 1000),
          tags: [],
          content: 'conformance tracer',
        },
        generateSecretKey()
      );
      const response = await fetch(`${relay.writeUrl}/write`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ event }),
      });
      expect(response.status).toBe(200);
      expect(await response.json()).toMatchObject({ eventId: event.id });

      const found = await query(relay.readWsUrl, { ids: [event.id] });
      expect(found.map((e) => e.id)).toEqual([event.id]);
    }
  );

  conformanceTest(
    'GET with Accept: application/nostr+json names the stub connector write edge',
    async () => {
      // The relay reads the edge in the background, so allow it to arrive.
      let document: Record<string, unknown> = {};
      const deadline = Date.now() + 20_000;
      do {
        const response = await fetch(relay.readUrl, {
          headers: { accept: 'application/nostr+json' },
        });
        expect(response.status).toBe(200);
        document = (await response.json()) as Record<string, unknown>;
        if (document['toon'] !== undefined) break;
        await new Promise((resolve) => setTimeout(resolve, 250));
      } while (Date.now() < deadline);

      expect(document['toon']).toMatchObject({
        ilp_address: STUB_ILP_ADDRESS,
        connector_url: STUB_WRITE_EDGE,
        connector_seal_key: STUB_SEAL_KEY,
        price: Number(STUB_PRICE),
      });
    }
  );
});
