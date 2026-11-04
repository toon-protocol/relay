import { afterAll, beforeAll, describe, expect } from 'vitest';
import { generateSecretKey, getPublicKey } from 'nostr-tools/pure';
import WebSocket from 'ws';
import { publishOk, sign } from './harness/wire.js';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { getDocument } from './harness/nip11.js';
import { conformanceTest, imageUnderTest } from './implementation.js';

/** The policy every /compute page carries: no script, and no remote media. */
const POLICY =
  "default-src 'self'; script-src 'none'; img-src 'self'; " +
  "form-action 'self'; base-uri 'none'";

let relay: RunningRelay;
let withoutCompute: RunningRelay;

const provider = generateSecretKey();
const stranger = generateSecretKey();

/** A Listing signed by `key`, joined to the Profile named by its `a` tag. */
const listing = (key: Uint8Array, d: string, content: object) =>
  sign(key, {
    kind: 30432,
    content: JSON.stringify(content),
    tags: [
      ['d', d],
      ['a', `10432:${getPublicKey(key)}:`],
      ['L', 'toon.network'],
    ],
  });

beforeAll(async () => {
  relay = await startRelay(imageUnderTest());
  withoutCompute = await startRelay(imageUnderTest(), {
    env: { TOON_UI_APPS: 'repo,social' },
  });
  await publishOk(
    relay,
    sign(provider, {
      kind: 10432,
      content: JSON.stringify({ name: 'Tidal Metal' }),
      tags: [['L', 'toon.network']],
    })
  );
  await publishOk(
    relay,
    listing(provider, 'small', {
      name: 'Small',
      resources: { vcpu: 2, memory: '4 GiB' },
      arch: 'arm64',
    })
  );
  await publishOk(
    relay,
    listing(provider, 'large', {
      name: 'Large',
      resources: { vcpu: 8, memory: '32 GiB' },
      arch: 'x86_64',
    })
  );
  await publishOk(relay, listing(stranger, 'lost', { name: 'Orphaned' }));
});

afterAll(async () => {
  await relay?.stop();
  await withoutCompute?.stop();
});

describe('relay image conformance: the /compute app UI', () => {
  conformanceTest(
    '/compute lists the Listings whose Profile the relay holds, and not a Listing with none',
    async () => {
      const response = await fetch(`${relay.readUrl}/compute`);
      const text = await response.text();
      expect(response.status).toBe(200);
      expect(response.headers.get('content-type')).toMatch(/^text\/html/);
      for (const shown of [
        'Tidal Metal',
        'Small',
        'Large',
        '32 GiB',
        'arm64',
      ]) {
        expect(text).toContain(shown);
      }
      expect(text).not.toContain('Orphaned');
    }
  );

  conformanceTest(
    'the page says it lists only Providers that publish to this relay',
    async () => {
      const text = await (await fetch(`${relay.readUrl}/compute`)).text();
      expect(text).toContain('only Providers that publish to this relay');
    }
  );

  conformanceTest(
    '/compute pages carry script-src none and no script',
    async () => {
      for (const path of ['/compute', '/compute/nope']) {
        const response = await fetch(`${relay.readUrl}${path}`);
        const text = await response.text();
        expect(response.headers.get('content-security-policy'), path).toBe(
          POLICY
        );
        expect(text).not.toMatch(/<script/i);
        expect(text).not.toMatch(/\son\w+=/);
        expect(text).not.toMatch(/\sstyle=/);
      }
    }
  );

  conformanceTest(
    'the site bar on /repo, /social and /compute links to /compute',
    async () => {
      for (const path of ['/repo', '/social', '/compute']) {
        const text = await (await fetch(`${relay.readUrl}${path}`)).text();
        expect(text, path).toMatch(/href="\/compute"[^>]*>Compute</);
      }
    }
  );

  conformanceTest(
    'with compute absent from TOON_UI_APPS, /compute returns 426 and /repo and /social draw no link to it',
    async () => {
      expect((await fetch(`${withoutCompute.readUrl}/compute`)).status).toBe(
        426
      );
      for (const path of ['/repo', '/social']) {
        const text = await (
          await fetch(`${withoutCompute.readUrl}${path}`)
        ).text();
        expect(text, path).not.toContain('/compute');
      }
    }
  );

  conformanceTest(
    'an unknown path under /compute is the 404 page',
    async () => {
      const response = await fetch(`${relay.readUrl}/compute/nope`);
      expect(response.status).toBe(404);
      expect(await response.text()).toContain('Not found');
    }
  );

  conformanceTest(
    'a WebSocket upgrade and the NIP-11 document on /compute still work',
    async () => {
      const upgraded = await new Promise<boolean>((resolve) => {
        const socket = new WebSocket(`${relay.readWsUrl}/compute`);
        socket.once('open', () => {
          socket.close();
          resolve(true);
        });
        socket.once('error', () => resolve(false));
      });
      expect(upgraded).toBe(true);
      const document = await getDocument(`${relay.readUrl}/compute`);
      expect(document.status).toBe(200);
      expect(document.body.pubkey).toMatch(/^[0-9a-f]{64}$/);
    }
  );
});
