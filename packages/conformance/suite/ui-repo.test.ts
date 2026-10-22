import { afterAll, beforeAll, describe, expect } from 'vitest';
import { nip19 } from 'nostr-tools';
import WebSocket from 'ws';
import { generateSecretKey } from 'nostr-tools/pure';
import { publishOk, pubkeyOf, sign } from './harness/wire.js';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { getDocument } from './harness/nip11.js';
import { conformanceTest, imageUnderTest } from './implementation.js';
import { commitOf, gitObject, treeOf } from './harness/git-fixture.js';

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
      // The list loads its own script from /repo/assets/; the seeded one is
      // escaped.
      expect(list).not.toContain('<script>alert');

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

  conformanceTest(
    'tree and blob pages list candidate URLs in order, with their own policy and the module from the manifest',
    async () => {
      const gateways = 'https://operator-gateway.test';
      const servers = 'https://operator-blossom.test';
      const configured = await startRelay(imageUnderTest(), {
        env: {
          TOON_UI_ARWEAVE_GATEWAYS: gateways,
          TOON_UI_BLOB_SERVERS: servers,
        },
      });
      try {
        const owner = generateSecretKey();
        const stranger = generateSecretKey();
        const file = gitObject('blob', 'hello\n');
        const root = treeOf([['100644', 'a b.txt', file]]);
        const commit = commitOf(root, 'first');
        const blobId = 'c'.repeat(64);
        await publishOk(
          configured,
          sign(owner, {
            kind: 30617,
            created_at: 10,
            tags: [
              ['d', 'layout'],
              ['blossoms', 'https://own-blossom.test'],
            ],
          })
        );
        const state = (key: Uint8Array, at: number, tags: string[][]) =>
          sign(key, {
            kind: 30618,
            created_at: at,
            tags: [['d', 'layout'], ...tags],
          });
        await publishOk(
          configured,
          state(owner, 20, [
            ['HEAD', 'ref: refs/heads/feature/x'],
            ['refs/heads/feature/x', commit.id],
            ['arweave', commit.id, 'txcommit', 'https://own-gateway.test'],
            ['blossom', commit.id, blobId],
            ['arweave', root.id, 'txroot'],
            ['arweave', file.id, 'txfile'],
          ])
        );
        // A stranger's state is not honoured, so its objects are not listed.
        await publishOk(
          configured,
          state(stranger, 30, [['arweave', 'f'.repeat(40), 'txevil']])
        );
        const home = `/repo/${nip19.npubEncode(pubkeyOf(owner))}/layout`;
        for (const [page, path, shown] of [
          ['tree', 'feature/x', []],
          ['blob', 'feature/x/a%20b.txt', ['a b.txt']],
        ] as const) {
          const response = await fetch(
            `${configured.readUrl}${home}/${page}/${path}`
          );
          expect(response.status).toBe(200);
          const policy = response.headers.get('content-security-policy') ?? '';
          expect(policy).toContain("default-src 'self'");
          expect(policy).toContain('connect-src https:');
          expect(policy).not.toContain('unsafe-inline');
          const html = await response.text();
          // One script, loaded from the manifest, and no inline code.
          const scripts = [...html.matchAll(/<script\b[^>]*>/g)].map(
            (m) => m[0]
          );
          expect(scripts).toHaveLength(1);
          expect(scripts[0]).toMatch(
            /^<script type="module" src="\/repo\/assets\/[\w.-]+\.js">$/
          );
          expect(html).not.toMatch(/\son\w+=/);
          const attribute = /data-git="([^"]*)"/.exec(html)?.[1] ?? '';
          const layout = JSON.parse(
            attribute
              .replaceAll('&quot;', '"')
              .replaceAll('&#39;', "'")
              .replaceAll('&lt;', '<')
              .replaceAll('&gt;', '>')
              .replaceAll('&amp;', '&')
          );
          expect(layout.kind).toBe(page);
          expect(layout.ref).toBe('feature/x');
          expect(layout.commit).toBe(commit.id);
          expect(layout.base).toBe(home);
          expect(layout.path).toEqual(shown);
          expect(layout.objects[commit.id]).toEqual([
            'https://own-gateway.test/txcommit',
            `https://own-blossom.test/${blobId}`,
            `${gateways}/txcommit`,
            `${servers}/${blobId}`,
          ]);
          expect(layout.objects[root.id]).toEqual([`${gateways}/txroot`]);
          expect(layout.objects['f'.repeat(40)]).toBeUndefined();
        }
        const missing = await fetch(`${configured.readUrl}${home}/blob/nope/a`);
        expect(missing.status).toBe(404);
      } finally {
        await configured.stop();
      }
    }
  );
});
