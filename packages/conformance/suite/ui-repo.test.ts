import { afterAll, beforeAll, describe, expect } from 'vitest';
import WebSocket from 'ws';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { getDocument } from './harness/nip11.js';
import { conformanceTest, imageUnderTest } from './implementation.js';

let relay: RunningRelay;
let off: RunningRelay;

beforeAll(async () => {
  relay = await startRelay(imageUnderTest());
  off = await startRelay(imageUnderTest(), { env: { TOON_UI_APPS: '' } });
});

afterAll(async () => {
  await relay?.stop();
  await off?.stop();
});

/** Open a WebSocket and report whether the upgrade completed. */
function upgrades(url: string): Promise<boolean> {
  return new Promise((resolve) => {
    const socket = new WebSocket(url);
    socket.once('open', () => {
      socket.close();
      resolve(true);
    });
    socket.once('error', () => resolve(false));
  });
}

describe('relay image conformance: the /repo app UI', () => {
  conformanceTest(
    'GET /repo returns a page with the Content-Security-Policy',
    async () => {
      const response = await fetch(`${relay.readUrl}/repo`);
      expect(response.status).toBe(200);
      expect(response.headers.get('content-type')).toMatch(/^text\/html/);
      expect(response.headers.get('content-security-policy')).toBe(
        "default-src 'self'"
      );
    }
  );

  conformanceTest('a plain GET / still returns 426', async () => {
    const response = await fetch(`${relay.readUrl}/`);
    expect(response.status).toBe(426);
  });

  conformanceTest('a WebSocket upgrade on /repo still works', async () => {
    expect(await upgrades(`${relay.readWsUrl}/repo`)).toBe(true);
  });

  conformanceTest('the NIP-11 document on /repo still works', async () => {
    const document = await getDocument(`${relay.readUrl}/repo`);
    expect(document.status).toBe(200);
    expect(document.headers.get('content-type')).toContain(
      'application/nostr+json'
    );
    expect(document.body.pubkey).toMatch(/^[0-9a-f]{64}$/);
  });

  conformanceTest('with TOON_UI_APPS empty, GET /repo returns 426', async () => {
    const response = await fetch(`${off.readUrl}/repo`);
    expect(response.status).toBe(426);
  });
});
