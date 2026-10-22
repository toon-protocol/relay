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
        "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'"
      );
    }
  );

  conformanceTest(
    'the page loads a hashed .wasm served as application/wasm, immutable',
    async () => {
      const page = await (await fetch(`${relay.readUrl}/repo`)).text();
      const script = /src="(\/repo\/assets\/[^"]+\.js)"/.exec(page)?.[1];
      expect(script, 'the page names its script').toBeDefined();
      const boot = await fetch(`${relay.readUrl}${script}`);
      expect(boot.status).toBe(200);
      expect(boot.headers.get('content-type')).toMatch(/javascript/);
      const glue = /'\.\/([^']+\.js)'/.exec(await boot.text())?.[1];
      expect(glue, 'the boot module names the loader').toBeDefined();
      const loader = await (
        await fetch(`${relay.readUrl}/repo/assets/${glue}`)
      ).text();
      const wasmName = /[\w.-]+\.[0-9a-f]{16}\.wasm/.exec(loader)?.[0];
      expect(wasmName, 'the loader names the hashed module').toBeDefined();
      const wasm = await fetch(`${relay.readUrl}/repo/assets/${wasmName}`);
      expect(wasm.status).toBe(200);
      expect(wasm.headers.get('content-type')).toBe('application/wasm');
      expect(wasm.headers.get('cache-control')).toBe(
        'public, max-age=31536000, immutable'
      );
    }
  );

  conformanceTest(
    'with TOON_UI_ASSETS_DIR unset, event pages work and /repo says browsing is unavailable',
    async () => {
      const bare = await startRelay(imageUnderTest(), {
        env: { TOON_UI_ASSETS_DIR: '' },
      });
      try {
        const page = await fetch(`${bare.readUrl}/repo`);
        expect(page.status).toBe(200);
        expect(await page.text()).toContain('git browsing unavailable');
        const document = await getDocument(`${bare.readUrl}/`);
        expect(document.status).toBe(200);
      } finally {
        await bare.stop();
      }
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

  conformanceTest(
    'with TOON_UI_APPS empty, GET /repo returns 426',
    async () => {
      const response = await fetch(`${off.readUrl}/repo`);
      expect(response.status).toBe(426);
    }
  );
});
