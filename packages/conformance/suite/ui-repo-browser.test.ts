import { execFile } from 'node:child_process';
import { createHash } from 'node:crypto';
import { once } from 'node:events';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { createServer, type Server } from 'node:https';
import type { AddressInfo } from 'node:net';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { promisify } from 'node:util';
import { nip19 } from 'nostr-tools';
import { generateSecretKey } from 'nostr-tools/pure';
import { chromium, type Browser } from 'playwright';
import { afterAll, beforeAll, describe, expect } from 'vitest';
import { commitOf, gitObject, treeOf } from './harness/git-fixture.js';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { pubkeyOf, publishOk, sign } from './harness/wire.js';
import { conformanceTest, imageUnderTest } from './implementation.js';

const run = promisify(execFile);

const HELD = 'this object is no longer held';

/** One request the stub blob server received. */
interface Received {
  path: string;
  origin: string | undefined;
  userAgent: string | undefined;
}

/**
 * The host the relay container reaches the host at; the browser is pointed at
 * loopback for the same name, so one URL serves both and a request from the
 * relay could land.
 */
const ALIAS = 'host.docker.internal';

/**
 * An https server standing in for an Arweave gateway or a Blossom server,
 * serving `objects` by path. It allows any origin (the browser fetches
 * cross-origin) and records every request, and the address of every
 * connection, so a case can tell who asked: the browser connects from
 * loopback, the relay container from the Docker bridge, even when its TLS
 * handshake fails before any request.
 */
async function stubGateway(objects: Map<string, Buffer>) {
  const directory = await mkdtemp(join(tmpdir(), 'stub-gateway-'));
  await run('openssl', [
    'req',
    '-x509',
    '-newkey',
    'rsa:2048',
    '-nodes',
    '-keyout',
    join(directory, 'key.pem'),
    '-out',
    join(directory, 'cert.pem'),
    '-days',
    '1',
    '-subj',
    '/CN=127.0.0.1',
    '-addext',
    `subjectAltName=IP:127.0.0.1,DNS:${ALIAS}`,
  ]);
  const received: Received[] = [];
  const connectedFrom: string[] = [];
  const server: Server = createServer(
    {
      key: await readFile(join(directory, 'key.pem')),
      cert: await readFile(join(directory, 'cert.pem')),
    },
    (request, response) => {
      connectedFrom.push(request.socket.remoteAddress ?? '');
      const path = request.url ?? '';
      received.push({
        path,
        origin: request.headers.origin,
        userAgent: request.headers['user-agent'],
      });
      response.setHeader('access-control-allow-origin', '*');
      const body = objects.get(path.slice(1));
      if (body === undefined) {
        response.statusCode = 404;
        response.end('no such transaction');
        return;
      }
      response.end(body);
    }
  );
  server.listen(0, '0.0.0.0');
  await once(server, 'listening');
  const { port } = server.address() as AddressInfo;
  return {
    url: `https://${ALIAS}:${port}`,
    received,
    connectedFrom,
    stop: async () => {
      server.close();
      await rm(directory, { recursive: true, force: true });
    },
  };
}

// A repository whose files are on the stub gateway, except one it lost, one
// it answers wrongly, and one held only on the stub Blossom server.
const hello = gitObject('blob', 'hello from the stub gateway\n');
const lost = gitObject('blob', 'this object was never uploaded\n');
const liar = gitObject('blob', 'what the liar should have served\n');
// Held only on Blossom, under the sha256 of its object bytes (header included).
const pooled = gitObject('blob', 'hello from the stub blossom server\n');
const pooledBlobId = createHash('sha256').update(pooled.headed).digest('hex');
// Entries in git's order, sorted by name.
const root = treeOf([
  ['100644', 'hello.txt', hello],
  ['100644', 'liar.txt', liar],
  ['100644', 'lost.txt', lost],
  ['100644', 'pooled.txt', pooled],
]);
const commit = commitOf(root, 'a commit');

// A later commit on another ref: hello.txt edited, liar.txt deleted, lost.txt
// renamed (its blob was never uploaded, so only its rename shows), one file
// added, pooled.txt unchanged.
const hello2 = gitObject('blob', 'hello again from the stub gateway\n');
const fresh = gitObject('blob', 'a new file\n');
const nextRoot = treeOf([
  ['100644', 'fresh.txt', fresh],
  ['100644', 'hello.txt', hello2],
  ['100644', 'moved.txt', lost],
  ['100644', 'pooled.txt', pooled],
]);
const next = commitOf(nextRoot, 'second commit', commit, 1700000100);

let relay: RunningRelay;
let gateway: Awaited<ReturnType<typeof stubGateway>>;
let blossom: Awaited<ReturnType<typeof stubGateway>>;
let browser: Browser;
let repo: string;

beforeAll(async () => {
  gateway = await stubGateway(
    new Map<string, Buffer>([
      [`tx${commit.id}`, commit.body],
      // Blossom-style: with the header.
      [`tx${root.id}`, root.headed],
      [`tx${hello.id}`, hello.body],
      [`tx${liar.id}`, Buffer.from('not what the tag says\n')],
      [`tx${next.id}`, next.body],
      [`tx${nextRoot.id}`, nextRoot.body],
      [`tx${hello2.id}`, hello2.body],
      [`tx${fresh.id}`, fresh.body],
    ])
  );
  blossom = await stubGateway(new Map([[pooledBlobId, pooled.headed]]));
  relay = await startRelay(imageUnderTest(), {
    env: {
      TOON_UI_ARWEAVE_GATEWAYS: gateway.url,
      TOON_UI_BLOB_SERVERS: blossom.url,
    },
  });
  const owner = generateSecretKey();
  await publishOk(
    relay,
    sign(owner, { kind: 30617, created_at: 10, tags: [['d', 'smoke']] })
  );
  await publishOk(
    relay,
    sign(owner, {
      kind: 30618,
      created_at: 20,
      tags: [
        ['d', 'smoke'],
        ['HEAD', 'ref: refs/heads/main'],
        ['refs/heads/main', commit.id],
        ['refs/heads/next', next.id],
        ...[commit, root, hello, lost, liar, next, nextRoot, hello2, fresh].map(
          (object) => ['arweave', object.id, `tx${object.id}`]
        ),
        ['blossom', pooled.id, pooledBlobId],
      ],
    })
  );
  repo = `${relay.readUrl}/repo/${nip19.npubEncode(pubkeyOf(owner))}/smoke`;
  browser = await chromium.launch({
    args: [`--host-resolver-rules=MAP ${ALIAS} 127.0.0.1`],
  });
});

afterAll(async () => {
  await browser?.close();
  await relay?.stop();
  await gateway?.stop();
  await blossom?.stop();
});

describe('relay image conformance: /repo git pages in a browser', () => {
  conformanceTest(
    'a blob page renders the file; a lost or wrong object shows "no longer held"; a blob only on Blossom renders; only the browser asks the gateway or Blossom server',
    async () => {
      // Fetching the pages without a browser must not make the relay fetch.
      for (const path of ['tree/main', 'blob/main/hello.txt']) {
        expect((await fetch(`${repo}/${path}`)).status).toBe(200);
      }
      expect(gateway.connectedFrom).toEqual([]);
      expect(blossom.connectedFrom).toEqual([]);

      const context = await browser.newContext({ ignoreHTTPSErrors: true });
      // What the browser saw, reported if the case fails: a page that never
      // renders says why only in its console.
      const seen: string[] = [];
      try {
        const page = await context.newPage();
        page.on('console', (m) =>
          seen.push(`console ${m.type()}: ${m.text()}`)
        );
        page.on('pageerror', (e) => seen.push(`page error: ${e.message}`));
        page.on('requestfailed', (r) =>
          seen.push(`failed: ${r.url()} ${r.failure()?.errorText ?? ''}`)
        );
        page.on('response', (r) => seen.push(`${r.status()} ${r.url()}`));
        const asked: string[] = [];
        page.on('request', (request) => {
          if (
            request.url().startsWith(gateway.url) ||
            request.url().startsWith(blossom.url)
          )
            asked.push(request.url());
        });

        await page.goto(`${repo}/blob/main/hello.txt`);
        await page.locator('#git pre').waitFor();
        expect(await page.locator('#git pre').textContent()).toBe(
          'hello from the stub gateway\n'
        );

        // A blob only on Blossom, fetched by its sha256, checked by sha1.
        await page.goto(`${repo}/blob/main/pooled.txt`);
        await page.locator('#git pre').waitFor();
        expect(await page.locator('#git pre').textContent()).toBe(
          'hello from the stub blossom server\n'
        );
        expect(blossom.received.map((r) => r.path)).toContain(
          `/${pooledBlobId}`
        );

        await page.goto(`${repo}/blob/main/lost.txt`);
        await page.locator('#git', { hasText: HELD }).waitFor();
        // The rest of the page still rendered.
        expect(await page.locator('h1').textContent()).toContain('smoke');

        await page.goto(`${repo}/blob/main/liar.txt`);
        await page.locator('#git', { hasText: HELD }).waitFor();
        expect(await page.locator('#git').textContent()).not.toContain(
          'not what the tag says'
        );

        await page.goto(`${repo}/tree/main`);
        await page.locator('#git li').first().waitFor();
        expect(await page.locator('#git li').allTextContents()).toEqual([
          'hello.txt',
          'liar.txt',
          'lost.txt',
          'pooled.txt',
        ]);

        // Everything the stubs received came from the browser, which the
        // browser itself saw and which carries the page's origin; no
        // connection came from the relay's container.
        for (const stub of [gateway, blossom]) {
          expect(stub.received.length).toBeGreaterThan(0);
          expect(new Set(stub.connectedFrom)).toEqual(new Set(['127.0.0.1']));
          for (const request of stub.received) {
            expect(request.origin).toBe(relay.readUrl);
            expect(request.userAgent).toMatch(/Chrome/);
          }
        }
        expect(
          new Set([
            ...gateway.received.map((r) => `${gateway.url}${r.path}`),
            ...blossom.received.map((r) => `${blossom.url}${r.path}`),
          ])
        ).toEqual(new Set(asked));
      } catch (error) {
        throw new Error(
          `${String(error)}\nthe browser saw:\n${seen.join('\n')}`
        );
      } finally {
        await context.close();
      }
    }
  );

  conformanceTest(
    'a commit page shows the diff against its parent, and the log lists the history',
    async () => {
      const context = await browser.newContext({ ignoreHTTPSErrors: true });
      const seen: string[] = [];
      try {
        const page = await context.newPage();
        page.on('console', (m) =>
          seen.push(`console ${m.type()}: ${m.text()}`)
        );
        page.on('pageerror', (e) => seen.push(`page error: ${e.message}`));

        await page.goto(`${repo}/commit/${next.id}`);
        await page.locator('#git h3').first().waitFor();
        expect(await page.locator('#git h3').allTextContents()).toEqual([
          'added: fresh.txt',
          'modified: hello.txt',
          'deleted: liar.txt',
          'renamed: lost.txt -> moved.txt',
        ]);
        const diffs = await page.locator('#git pre').allTextContents();
        expect(diffs[0]).toBe('second commit\n');
        expect(diffs).toContain('+a new file\n');
        expect(diffs).toContain(
          '-hello from the stub gateway\n+hello again from the stub gateway\n'
        );
        // liar.txt's blob is served wrongly, so its diff is not shown.
        expect(await page.locator('#git').textContent()).toContain(HELD);
        expect(await page.locator('#git').textContent()).not.toContain(
          'not what the tag says'
        );
        // The parent links to its own commit page.
        expect(
          await page.locator(`#git a[href$="/commit/${commit.id}"]`).count()
        ).toBe(1);

        await page.goto(`${repo}/commits/next`);
        await page.locator('#git li').first().waitFor();
        const rows = await page.locator('#git li').allTextContents();
        expect(rows).toHaveLength(2);
        expect(rows[0]).toContain('second commit');
        expect(rows[1]).toContain('a commit');
      } catch (error) {
        throw new Error(
          `${String(error)}\nthe browser saw:\n${seen.join('\n')}`
        );
      } finally {
        await context.close();
      }
    }
  );
});
