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
    'issue and pull pages show threads and the honoured status',
    async () => {
      const owner = generateSecretKey();
      const author = generateSecretKey();
      const stranger = generateSecretKey();
      const ownerKey = pubkeyOf(owner);
      const coordinate = `30617:${ownerKey}:threads`;
      await publishOk(
        relay,
        sign(owner, {
          kind: 30617,
          created_at: 10,
          tags: [
            ['d', 'threads'],
            ['name', 'Threads'],
          ],
        })
      );
      const issue = sign(author, {
        kind: 1621,
        created_at: 20,
        content: 'Broken <script>alert(1)</script> thing',
        tags: [
          ['a', coordinate],
          ['subject', 'Crash <b>on start</b>'],
        ],
      });
      await publishOk(relay, issue);
      const other = sign(author, {
        kind: 1621,
        created_at: 21,
        content: 'Second issue',
        tags: [['a', coordinate]],
      });
      await publishOk(relay, other);
      await publishOk(
        relay,
        sign(author, {
          kind: 1111,
          created_at: 30,
          content: 'nip22 comment',
          tags: [
            ['E', issue.id],
            ['K', '1621'],
          ],
        })
      );
      await publishOk(
        relay,
        sign(author, {
          kind: 1622,
          created_at: 31,
          content: 'legacy comment',
          tags: [
            ['a', coordinate],
            ['e', issue.id, '', 'root'],
          ],
        })
      );
      await publishOk(
        relay,
        sign(owner, {
          kind: 1631,
          created_at: 40,
          tags: [['e', issue.id]],
        })
      );
      // A stranger's status is ignored; the target's own author is honoured.
      await publishOk(
        relay,
        sign(stranger, {
          kind: 1632,
          created_at: 50,
          tags: [['e', issue.id]],
        })
      );
      await publishOk(
        relay,
        sign(stranger, {
          kind: 1632,
          created_at: 50,
          tags: [['e', other.id]],
        })
      );

      const patch = sign(author, {
        kind: 1617,
        created_at: 60,
        content: 'diff --git a/x b/x\n+<script>evil()</script>',
        tags: [
          ['a', coordinate],
          ['t', 'root'],
        ],
      });
      await publishOk(relay, patch);
      const reply = sign(author, {
        kind: 1617,
        created_at: 61,
        content: 'follow-up patch',
        tags: [
          ['a', coordinate],
          ['e', patch.id, '', 'reply'],
        ],
      });
      await publishOk(relay, reply);
      const pr = sign(author, {
        kind: 1618,
        created_at: 62,
        content: 'Please pull',
        tags: [
          ['a', coordinate],
          ['subject', 'A pull request'],
          ['c', 'c'.repeat(40)],
        ],
      });
      await publishOk(relay, pr);
      await publishOk(
        relay,
        sign(author, {
          kind: 1619,
          created_at: 70,
          tags: [
            ['a', coordinate],
            ['E', pr.id],
            ['c', 'd'.repeat(40)],
          ],
        })
      );
      await publishOk(
        relay,
        sign(author, {
          kind: 1632,
          created_at: 71,
          tags: [['e', patch.id]],
        })
      );

      const base = `${relay.readUrl}/repo/${nip19.npubEncode(ownerKey)}/threads`;
      const get = async (path: string) => {
        const response = await fetch(`${base}${path}`);
        expect(response.headers.get('content-security-policy')).toBe(
          "default-src 'self'; script-src 'self' 'wasm-unsafe-eval'"
        );
        const text = await response.text();
        expect(text).not.toContain('<script');
        return { status: response.status, text };
      };

      const home = await get('');
      expect(home.text).toContain('/issues"');
      expect(home.text).toContain('/pulls"');

      const issues = await get('/issues');
      expect(issues.status).toBe(200);
      expect(issues.text).toContain(`/issues/${issue.id}"`);
      expect(issues.text).toContain('Crash &lt;b&gt;on start&lt;/b&gt;');
      expect(issues.text).toContain('[resolved]');
      expect(issues.text).toContain(`/issues/${other.id}"`);
      expect(issues.text).toContain('[open]');

      const thread = await get(`/issues/${issue.id}`);
      expect(thread.status).toBe(200);
      expect(thread.text).toContain('Status: <strong>resolved</strong>');
      expect(thread.text).toContain('Broken &lt;script&gt;');
      expect(thread.text).toContain(nip19.npubEncode(pubkeyOf(author)));
      expect(thread.text.indexOf('nip22 comment')).toBeGreaterThan(0);
      expect(thread.text.indexOf('legacy comment')).toBeGreaterThan(
        thread.text.indexOf('nip22 comment')
      );
      const ignored = await get(`/issues/${other.id}`);
      expect(ignored.text).toContain('Status: <strong>open</strong>');

      const pulls = await get('/pulls');
      expect(pulls.status).toBe(200);
      expect(pulls.text).toContain(`/pulls/${patch.id}"`);
      expect(pulls.text).toContain(`/pulls/${pr.id}"`);
      expect(pulls.text).not.toContain(`/pulls/${reply.id}`);
      expect(pulls.text).toContain('A pull request');

      const patchPage = await get(`/pulls/${patch.id}`);
      expect(patchPage.status).toBe(200);
      expect(patchPage.text).toContain('+&lt;script&gt;evil()&lt;/script&gt;');
      // The patch's own author closed it.
      expect(patchPage.text).toContain('Status: <strong>closed</strong>');

      const prPage = await get(`/pulls/${pr.id}`);
      expect(prPage.status).toBe(200);
      expect(prPage.text).toContain('Latest tip: <code>' + 'd'.repeat(40));
      expect(prPage.text).toContain('Status: <strong>open</strong>');

      // Not the right kind, malformed, unknown, or another route's id.
      for (const path of [
        `/issues/${patch.id}`,
        `/pulls/${issue.id}`,
        '/issues/not-an-id',
        `/issues/${'0'.repeat(64)}`,
        `/pulls/${'0'.repeat(64)}`,
      ]) {
        expect((await get(path)).status).toBe(404);
      }
      // A thread never resolves under another repo, even one that exists.
      await publishOk(
        relay,
        sign(stranger, {
          kind: 30617,
          created_at: 80,
          tags: [
            ['d', 'threads'],
            ['name', 'Elsewhere'],
          ],
        })
      );
      const elsewhereBase = `${relay.readUrl}/repo/${nip19.npubEncode(pubkeyOf(stranger))}/threads`;
      expect((await fetch(`${elsewhereBase}/issues`)).status).toBe(200);
      const elsewhere = await fetch(`${elsewhereBase}/issues/${issue.id}`);
      expect(elsewhere.status).toBe(404);
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
    'tree, blob, commits and commit pages list candidate URLs in order, with their own policy and the module from the manifest',
    async () => {
      const gateways = 'https://operator-gateway.test';
      const servers = 'https://operator-blossom.test';
      const configured = await startRelay(imageUnderTest(), {
        env: {
          TOON_UI_ARWEAVE_GATEWAYS: gateways,
          TOON_UI_BLOB_SERVERS: servers,
        },
      });
      // The page's `data-git` attribute, parsed.
      const layoutOf = (html: string) =>
        JSON.parse(
          (/data-git="([^"]*)"/.exec(html)?.[1] ?? '')
            .replaceAll('&quot;', '"')
            .replaceAll('&#39;', "'")
            .replaceAll('&lt;', '<')
            .replaceAll('&gt;', '>')
            .replaceAll('&amp;', '&')
        );
      try {
        const owner = generateSecretKey();
        const stranger = generateSecretKey();
        const file = gitObject('blob', 'hello\n');
        const root = treeOf([['100644', 'a b.txt', file]]);
        const commit = commitOf(root, 'first');
        const blobId = 'c'.repeat(64);
        const fileBlobId = 'd'.repeat(64);
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
            ['blossom', file.id, fileBlobId],
          ])
        );
        // A stranger's state is not honoured, so its objects are not listed.
        await publishOk(
          configured,
          state(stranger, 30, [['arweave', 'f'.repeat(40), 'txevil']])
        );
        const home = `/repo/${nip19.npubEncode(pubkeyOf(owner))}/layout`;
        for (const [page, path, shown, ref] of [
          ['tree', 'tree/feature/x', [], 'feature/x'],
          ['blob', 'blob/feature/x/a%20b.txt', ['a b.txt'], 'feature/x'],
          ['commits', 'commits/feature/x', [], 'feature/x'],
          ['commits', 'commits', [], 'feature/x'],
          ['commit', `commit/${commit.id}`, [], commit.id],
        ] as const) {
          const response = await fetch(`${configured.readUrl}${home}/${path}`);
          expect(response.status).toBe(200);
          const policy = response.headers.get('content-security-policy') ?? '';
          expect(policy).toContain("default-src 'self'");
          expect(policy).toContain("connect-src 'self' https:");
          expect(policy).not.toContain('unsafe-inline');
          const html = await response.text();
          // One script, loaded from the manifest, and no inline code.
          const scripts = [...html.matchAll(/<script\b[^>]*>/gi)].map(
            (m) => m[0]
          );
          expect(scripts).toHaveLength(1);
          expect(scripts[0]).toMatch(
            /^<script type="module" src="\/repo\/assets\/[\w.-]+\.js">$/
          );
          expect(html).not.toMatch(/\son\w+=/);
          const layout = layoutOf(html);
          expect(layout.kind).toBe(page);
          expect(layout.ref).toBe(ref);
          expect(layout.commit).toBe(commit.id);
          expect(layout.base).toBe(home);
          expect(layout.path).toEqual(shown);
          expect(layout.objects[commit.id]).toEqual([
            `https://own-blossom.test/${blobId}`,
            `${servers}/${blobId}`,
            'https://own-gateway.test/txcommit',
            `${gateways}/txcommit`,
          ]);
          expect(layout.objects[root.id]).toEqual([`${gateways}/txroot`]);
          // Only a `blossom` tag: Blossom servers alone.
          expect(layout.objects[file.id]).toEqual([
            `https://own-blossom.test/${fileBlobId}`,
            `${servers}/${fileBlobId}`,
          ]);
          expect(layout.objects['f'.repeat(40)]).toBeUndefined();
        }

        // A repository whose state carries only `blossom` tags: every object
        // is listed against Blossom servers alone, the announcement's first.
        const pooledFile = gitObject('blob', 'only on blossom\n');
        const pooledRoot = treeOf([['100644', 'b.txt', pooledFile]]);
        const pooledCommit = commitOf(pooledRoot, 'pooled');
        const pooled = [pooledCommit, pooledRoot, pooledFile].map(
          (object, i) => [object.id, String(i + 1).repeat(64)] as const
        );
        await publishOk(
          configured,
          sign(owner, {
            kind: 30617,
            created_at: 10,
            tags: [
              ['d', 'pooled'],
              ['blossoms', 'https://own-blossom.test'],
            ],
          })
        );
        await publishOk(
          configured,
          sign(owner, {
            kind: 30618,
            created_at: 20,
            tags: [
              ['d', 'pooled'],
              ['HEAD', 'ref: refs/heads/main'],
              ['refs/heads/main', pooledCommit.id],
              ...pooled.map(([id, blob]) => ['blossom', id, blob]),
            ],
          })
        );
        const pooledHome = `/repo/${nip19.npubEncode(pubkeyOf(owner))}/pooled`;
        const pooledPage = await fetch(
          `${configured.readUrl}${pooledHome}/tree/main`
        );
        expect(pooledPage.status).toBe(200);
        const pooledLayout = layoutOf(await pooledPage.text());
        expect(pooledLayout.commit).toBe(pooledCommit.id);
        expect(pooledLayout.objects).toEqual(
          Object.fromEntries(
            pooled.map(([id, blob]) => [
              id,
              [`https://own-blossom.test/${blob}`, `${servers}/${blob}`],
            ])
          )
        );

        const missing = await fetch(`${configured.readUrl}${home}/blob/nope/a`);
        expect(missing.status).toBe(404);
        // A commit page wants a full commit id; a log wants a ref that exists.
        for (const path of ['commit/main', 'commits/nope']) {
          expect(
            (await fetch(`${configured.readUrl}${home}/${path}`)).status
          ).toBe(404);
        }
        // The log's page number reaches the layout, and nonsense is page 0.
        const layoutOf = async (path: string) => {
          const html = await (
            await fetch(`${configured.readUrl}${home}/${path}`)
          ).text();
          const attribute = /data-git="([^"]*)"/.exec(html)?.[1] ?? '';
          return JSON.parse(attribute.replaceAll('&quot;', '"'));
        };
        expect((await layoutOf('commits/feature/x?page=2')).page).toBe(2);
        expect((await layoutOf('commits/feature/x?page=-1')).page).toBe(0);
      } finally {
        await configured.stop();
      }
    }
  );
});
