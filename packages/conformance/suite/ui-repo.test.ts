import { afterAll, beforeAll, describe, expect } from 'vitest';
import { nip19 } from 'nostr-tools';
import WebSocket from 'ws';
import { generateSecretKey } from 'nostr-tools/pure';
import { publishOk, pubkeyOf, sign } from './harness/wire.js';
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

  conformanceTest(
    'with TOON_UI_APPS empty, GET /repo returns 426',
    async () => {
      const response = await fetch(`${off.readUrl}/repo`);
      expect(response.status).toBe(426);
    }
  );

  conformanceTest(
    'the list, the home page and the redirects read seeded events',
    async () => {
      const owner = generateSecretKey();
      const maintainer = generateSecretKey();
      const stranger = generateSecretKey();
      const ownerKey = pubkeyOf(owner);
      const announcement = sign(owner, {
        kind: 30617,
        created_at: 10,
        tags: [
          ['d', 'proj'],
          ['name', 'Seeded <b>'],
          ['description', 'A <script>alert(1)</script> repo'],
          ['maintainers', pubkeyOf(maintainer)],
          ['clone', 'https://example.test/proj.git', 'javascript:alert(1)'],
        ],
      });
      await publishOk(relay, announcement);
      await publishOk(
        relay,
        sign(owner, {
          kind: 0,
          content: JSON.stringify({ display_name: 'Ada Owner' }),
        })
      );
      await publishOk(
        relay,
        sign(maintainer, {
          kind: 30618,
          created_at: 20,
          tags: [
            ['d', 'proj'],
            ['HEAD', 'ref: refs/heads/main'],
            ['refs/heads/main', 'a'.repeat(40)],
          ],
        })
      );
      await publishOk(
        relay,
        sign(stranger, {
          kind: 30618,
          created_at: 30,
          tags: [
            ['d', 'proj'],
            ['refs/heads/evil', 'b'.repeat(40)],
          ],
        })
      );
      const issue = sign(stranger, {
        kind: 1621,
        tags: [['a', `30617:${ownerKey}:proj`]],
      });
      await publishOk(relay, issue);

      const list = await (await fetch(`${relay.readUrl}/repo`)).text();
      expect(list).toContain('Seeded &lt;b&gt;');
      expect(list).toContain('Ada Owner');
      expect(list).not.toContain('<script');

      const home = `/repo/${nip19.npubEncode(ownerKey)}/proj`;
      const page = await fetch(`${relay.readUrl}${home}`);
      expect(page.status).toBe(200);
      const html = await page.text();
      expect(html).toContain('A &lt;script&gt;alert(1)&lt;/script&gt; repo');
      expect(html).toContain('Ada Owner');
      expect(html).toContain(nip19.npubEncode(pubkeyOf(maintainer)));
      expect(html).toContain('<a href="https://example.test/proj.git"');
      expect(html).not.toContain('href="javascript:');
      expect(html).toContain('Default branch: <code>main</code>');
      expect(html).toContain('refs/heads/main');
      expect(html).not.toContain('evil');
      expect(html).not.toContain('<script');

      const naddr = nip19.naddrEncode({
        kind: 30617,
        pubkey: ownerKey,
        identifier: 'proj',
      });
      for (const target of [naddr, announcement.id, issue.id]) {
        const redirect = await fetch(`${relay.readUrl}/repo/${target}`, {
          redirect: 'manual',
        });
        expect(redirect.status).toBe(301);
        expect(redirect.headers.get('location')).toBe(home);
      }
    }
  );

  conformanceTest(
    'an unknown repo or id returns 404 HTML, not 426',
    async () => {
      for (const path of [
        `/repo/${'0'.repeat(64)}`,
        `/repo/${nip19.npubEncode(pubkeyOf(generateSecretKey()))}/none`,
      ]) {
        const response = await fetch(`${relay.readUrl}${path}`);
        expect(response.status).toBe(404);
        expect(response.headers.get('content-type')).toMatch(/^text\/html/);
        expect(await response.text()).not.toContain('<script');
      }
    }
  );
});
