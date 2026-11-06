import { afterAll, beforeAll, describe, expect } from 'vitest';
import { nip19 } from 'nostr-tools';
import { generateSecretKey, getPublicKey } from 'nostr-tools/pure';
import WebSocket from 'ws';
import { now, publishOk, sign } from './harness/wire.js';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { getDocument } from './harness/nip11.js';
import {
  conformanceTest,
  conformanceTestEach,
  imageUnderTest,
} from './implementation.js';

/** The policy every /compute page carries: no script, and no remote media. */
const POLICY =
  "default-src 'self'; script-src 'none'; img-src 'self'; " +
  "form-action 'self'; base-uri 'none'";

let relay: RunningRelay;
let withoutCompute: RunningRelay;

const provider = generateSecretKey();
const stranger = generateSecretKey();
/** Live, with two Listings. */
const up = generateSecretKey();
/** No Liveness, with the cheapest Listing. */
const down = generateSecretKey();
/** A Listing labelled arm64, vm, a100 and docker. */
const open = generateSecretKey();
/** A Hidden Provider, whose host is never shown. */
const hidden = generateSecretKey();
/** A /social profile with no Provider Profile. */
const plain = generateSecretKey();

const npub = (key: Uint8Array) => nip19.npubEncode(getPublicKey(key));

/** A Profile signed by `key`, labelled as TOON Compute's. */
const profile = (key: Uint8Array, content: object = {}) =>
  sign(key, {
    kind: 10432,
    content: JSON.stringify(content),
    tags: [['L', 'toon.network']],
  });

/** A Listing signed by `key`, joined to the Profile named by its `a` tag. */
const listing = (
  key: Uint8Array,
  d: string,
  content: object,
  labels: string[][] = []
) =>
  sign(key, {
    kind: 30432,
    content: JSON.stringify(content),
    tags: [
      ['d', d],
      ['a', `10432:${getPublicKey(key)}:`],
      ['L', 'toon.network'],
      ...labels,
    ],
  });

/** GET `path` from the relay under test, which must answer 200 with HTML. */
async function html(path: string, on: RunningRelay = relay): Promise<string> {
  const response = await fetch(`${on.readUrl}${path}`);
  expect(response.status, path).toBe(200);
  return response.text();
}

/** The directory row of the Listing called `name`, or undefined. */
function row(page: string, name: string): string | undefined {
  const at = page.indexOf(`data-listing="${name}"`);
  if (at < 0) return undefined;
  const rest = page.slice(at);
  return rest.slice(0, rest.indexOf('</tr>'));
}

beforeAll(async () => {
  relay = await startRelay(imageUnderTest());
  withoutCompute = await startRelay(imageUnderTest(), {
    env: { TOON_UI_APPS: 'repo,social' },
  });
  await publishOk(
    relay,
    profile(provider, {
      name: 'Tidal Metal',
      isolation: 'vm',
      settlement: [{ chain: 'base', tokens: ['USDC'] }],
      relay_set: ['wss://relay.example'],
    })
  );
  await publishOk(
    relay,
    listing(provider, 'small', {
      name: 'Small',
      resources: { vcpu: 2, memory: '4 GiB' },
      arch: 'arm64',
      price: 5,
      lease_interval_s: 60,
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

  // Sorting: a live Provider with two Listings, and a cheaper one that is down.
  for (const key of [up, down, open, hidden]) {
    await publishOk(
      relay,
      profile(key, key === hidden ? { host: 'secret.example.net' } : {})
    );
  }
  await publishOk(
    relay,
    listing(down, 'cheap-down', {
      name: 'CheapDown',
      price: 1,
      lease_interval_s: 3600,
    })
  );
  await publishOk(
    relay,
    listing(up, 'dear-up', {
      name: 'DearUp',
      price: 1000,
      lease_interval_s: 7,
      standby_price: 40,
    })
  );
  await publishOk(
    relay,
    listing(up, 'cheap-up', {
      name: 'CheapUp',
      price: 50,
      lease_interval_s: 60,
    })
  );
  await publishOk(
    relay,
    sign(up, {
      kind: 10433,
      content: JSON.stringify({ available: { 'cheap-up': 3 } }),
      tags: [
        ['L', 'toon.network'],
        ['expiration', String(now() + 3600)],
      ],
    })
  );

  // Filtering: an open Provider and a Hidden one.
  await publishOk(
    relay,
    listing(open, 'open', { name: 'Open', host: 'secret.example.net' }, [
      ['l', 'arch:arm64'],
      ['l', 'isolation:vm'],
      ['l', 'gpu:a100'],
      ['t', 'docker'],
    ])
  );
  await publishOk(
    relay,
    listing(hidden, 'hid', { name: 'Hid', host: 'secret.example.net' }, [
      ['l', 'arch:x86_64'],
      ['l', 'isolation:container'],
      ['l', 'hidden:true'],
      ['t', 'docker'],
    ])
  );

  // A /social profile with no Provider Profile behind it.
  await publishOk(withoutCompute, profile(provider));
  await publishOk(relay, sign(plain, { kind: 0, content: '{}' }));
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
      expect(response.headers.get('content-type')).toContain('text/html');
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

  conformanceTest(
    'live Providers come first, then the cheapest per hour, with exact prices and capacity',
    async () => {
      const page = await html('/compute');
      const at = (name: string) => page.indexOf(`data-listing="${name}"`);
      expect(at('CheapUp')).toBeGreaterThan(-1);
      expect(at('CheapUp')).toBeLessThan(at('DearUp'));
      expect(at('DearUp')).toBeLessThan(at('CheapDown'));

      const cheapUp = row(page, 'CheapUp') ?? '';
      expect(cheapUp).toContain('live, 3 available');
      expect(cheapUp).toContain('50 µUSDC per 60 s (3000 µUSDC per hour)');
      expect(cheapUp).toContain('no standbys');
      const dearUp = row(page, 'DearUp') ?? '';
      expect(dearUp).toContain('data-status="live"');
      expect(dearUp).toContain(
        '1000 µUSDC per 7 s (514285 + 5/7 µUSDC per hour)'
      );
      expect(dearUp).toContain('<td>40 µUSDC</td>');
      const cheapDown = row(page, 'CheapDown') ?? '';
      expect(cheapDown).toContain('data-status="down"');
      expect(cheapDown).toContain('1 µUSDC per 3600 s (1 µUSDC per hour)');
    }
  );

  conformanceTest(
    "a live Provider's page says when its current Liveness was signed",
    async () => {
      const live = await html(`/compute/p/${npub(up)}`);
      expect(live).toContain('data-status="live"');
      expect(live).toContain('Liveness signed');
      const gone = await html(`/compute/p/${npub(down)}`);
      expect(gone).toContain('data-status="down"');
      expect(gone).not.toContain('data-live-since');
    }
  );

  conformanceTestEach(
    [
      ['', true, true],
      ['?arch=arm64', true, false],
      ['?isolation=container', false, true],
      ['?gpu=a100', true, false],
      ['?cap=docker', true, true],
      ['?hidden=true', false, true],
      ['?hidden=false', true, false],
      ['?arch=arm64&hidden=true', false, false],
      [
        '?arch=arm64&isolation=vm&gpu=a100&cap=docker&hidden=false',
        true,
        false,
      ],
    ] as const,
    ([query]) => `/compute${query} filters the directory`,
    async ([query, openShown, hiddenShown]) => {
      const page = await html(`/compute${query}`);
      expect(row(page, 'Open') !== undefined, 'Open').toBe(openShown);
      expect(row(page, 'Hid') !== undefined, 'Hid').toBe(hiddenShown);
    }
  );

  conformanceTest(
    'the filter form works without script, each row shows what its filters match, and a Hidden Provider is Anyone-only with no host',
    async () => {
      const page = await html('/compute?arch=x86_64');
      expect(page).toContain('method="get"');
      expect(page).toContain('value="x86_64"');
      expect(row(page, 'Hid')).toContain('Anyone-only');
      const open = row(await html('/compute'), 'Open') ?? '';
      for (const shown of ['<td>vm</td>', 'gpu a100', '<td>docker</td>']) {
        expect(open).toContain(shown);
      }
      for (const path of [
        '/compute',
        '/compute?arch=x86_64',
        `/compute/p/${npub(hidden)}`,
        `/compute/${npub(hidden)}/hid`,
      ]) {
        expect(await html(path), path).not.toContain('secret.example.net');
      }
      expect(await html(`/compute/p/${npub(hidden)}`)).toContain('Anyone-only');
    }
  );

  conformanceTest(
    'a Provider page shows its Profile and Listings and links to its /social profile',
    async () => {
      const page = await html(`/compute/p/${npub(provider)}`);
      for (const shown of [
        'Tidal Metal',
        'vm',
        'base: USDC',
        'wss://relay.example',
        '5 µUSDC per 60 s',
        `href="/compute/${npub(provider)}/small"`,
        `href="/social/p/${npub(provider)}"`,
      ]) {
        expect(page).toContain(shown);
      }
      const one = await html(`/compute/${npub(provider)}/small`);
      expect(one).toContain('vcpu 2');
      expect(one).toContain(`href="/compute/p/${npub(provider)}"`);
      const index = await html('/compute');
      expect(index).toContain(`href="/compute/p/${npub(provider)}"`);
      expect(index).toContain(`href="/compute/${npub(provider)}/small"`);
    }
  );

  conformanceTest(
    "a Listing's naddr redirects to its page, and an unknown Provider or Listing is the 404 page",
    async () => {
      const naddr = nip19.naddrEncode({
        kind: 30432,
        pubkey: getPublicKey(provider),
        identifier: 'small',
      });
      const response = await fetch(`${relay.readUrl}/compute/${naddr}`, {
        redirect: 'manual',
      });
      expect(response.status).toBeGreaterThanOrEqual(300);
      expect(response.status).toBeLessThan(400);
      expect(response.headers.get('location')).toBe(
        `/compute/${npub(provider)}/small`
      );
      for (const path of [
        `/compute/${npub(provider)}/none`,
        `/compute/p/${npub(generateSecretKey())}`,
        `/compute/p/${npub(stranger)}`,
      ]) {
        const missing = await fetch(`${relay.readUrl}${path}`);
        expect(missing.status, path).toBe(404);
        expect(await missing.text(), path).toContain('Not found');
      }
    }
  );

  conformanceTest(
    'a /social profile carries the Compute provider badge only for a Provider, and only while compute is on',
    async () => {
      const badge = `href="/compute/p/${npub(provider)}"`;
      expect(await html(`/social/p/${npub(provider)}`)).toContain(badge);
      expect(await html(`/social/p/${npub(plain)}`)).not.toContain(
        'data-compute-provider'
      );
      expect(
        await html(`/social/p/${npub(provider)}`, withoutCompute)
      ).not.toContain('/compute');
    }
  );

  conformanceTest(
    'a Listing page works out the lease for an agent: intervals, an exact total, and a request to copy',
    async () => {
      const path = `/compute/${npub(provider)}/small`;
      const naddr = nip19.naddrEncode({
        kind: 30432,
        pubkey: getPublicKey(provider),
        identifier: 'small',
      });
      const empty = await html(path);
      expect(empty).toContain('Use with an agent');
      expect(empty).not.toContain('data-agent-request');
      expect(empty).toContain('a spawn would fail until the Provider is live');

      const two = await fetch(`${relay.readUrl}${path}?hours=2&image=nginx`);
      expect(two.status).toBe(200);
      expect(two.headers.get('content-security-policy')).toBe(POLICY);
      const text = await two.text();
      expect(text).toContain('120 Lease Intervals, 600 µUSDC');
      expect(text).toContain(`Run \`nginx\` on Listing \`${naddr}\``);
      expect(text).toContain('port &lt;port&gt;');
      expect(text).toContain('at most 600 µUSDC');
      expect(text).not.toContain('Warm Standby');
      const five = await html(`${path}?intervals=5`);
      expect(five).toContain('5 × 1 min Lease Intervals');
      expect(five).toContain('25 µUSDC');
      expect(five).toContain('Run `&lt;image&gt;`');

      // A live Provider, a 7 s interval and a Warm Standby.
      const dear = await html(`/compute/${npub(up)}/dear-up?hours=2`);
      expect(dear).not.toContain('a spawn would fail');
      expect(dear).toContain('1029 Lease Intervals, 1029000 µUSDC');
      expect(dear).toContain('41160 µUSDC for one Warm Standby');
      expect(await html(`/compute/${npub(up)}/dear-up?intervals=5`)).toContain(
        '200 µUSDC for one Warm Standby'
      );

      const script = await html(`${path}?hours=1&image=%3Cscript%3E`);
      expect(script).toContain('&lt;script&gt;');
      expect(script).not.toContain('<script>');
      for (const query of [
        'hours=abc',
        'hours=0',
        'hours=-1',
        'hours=721',
        'hours=1&intervals=2',
        'hours=18446744073709551615',
        'intervals=0',
        'intervals=43201',
        'intervals=99999999999999999999',
      ]) {
        const page = await html(`${path}?${query}`);
        expect(page, query).toContain('data-agent-note');
        expect(page, query).not.toContain('data-agent-request');
      }
      const none = await html(`/compute/${npub(provider)}/large?hours=1`);
      expect(none).toContain('states no price or interval');
    }
  );
});
