import { afterAll, beforeAll, describe, expect } from 'vitest';
import { nip19 } from 'nostr-tools';
import { generateSecretKey, getPublicKey } from 'nostr-tools/pure';
import { now, publishOk, sign } from './harness/wire.js';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { conformanceTest, imageUnderTest } from './implementation.js';

/**
 * Deployments on /compute (#330, ADR 0012, the Deployment NIP draft): a
 * Deployment's page and state, the Provider page's Serving list and activity
 * log, the Environments tab on /repo, and the link-preview cards of the
 * /compute pages. Every state comes only from public events, and only from
 * keys the Deployment names.
 */
const ORIGIN = 'https://relay.example';

/** The policy every /compute page carries: no script, and no remote media. */
const POLICY =
  "default-src 'self'; script-src 'none'; img-src 'self'; " +
  "form-action 'self'; base-uri 'none'";

let relay: RunningRelay;

const owner = generateSecretKey();
const maintainer = generateSecretKey();
const stranger = generateSecretKey();
/** No Liveness: down. */
const sleepy = generateSecretKey();
/** Live. */
const primary = generateSecretKey();
const standby = generateSecretKey();
const other = generateSecretKey();

const npub = (key: Uint8Array) => nip19.npubEncode(getPublicKey(key));
const workload = (n: number) => n.toString(16).padStart(2, '0').repeat(32);

const COMMIT = '1'.repeat(40);
const IMAGE = 'cd'.repeat(32);

/**
 * A Deployment signed by `tenant` of `owner`'s `site` repository, `env`, on
 * `on` with `standby` behind it, whose workload id is `id`.
 */
function deployment(
  tenant: Uint8Array,
  env: string,
  on: Uint8Array,
  id: string,
  options: { expiration?: number; extra?: string[][] } = {}
) {
  return sign(tenant, {
    kind: 30437,
    tags: [
      ['d', env],
      ['a', `30617:${getPublicKey(owner)}:site`],
      ['c', COMMIT],
      ['a', `30432:${getPublicKey(on)}:small`],
      ['p', getPublicKey(on), 'primary'],
      ['p', getPublicKey(standby), 'standby'],
      ['x', id, 'workload'],
      ['x', IMAGE, 'image'],
      ['r', 'https://app.example.com'],
      ['expiration', String(options.expiration ?? now() + 3600)],
      ['L', 'toon.network'],
      ...(options.extra ?? []),
    ],
  });
}

/** An Eviction Notice of workload `id`, signed by `key`, `ago` seconds old. */
const eviction = (key: Uint8Array, id: string, content: object, ago = 0) =>
  sign(key, {
    kind: 4433,
    created_at: now() - ago,
    content: JSON.stringify(content),
    tags: [
      ['x', id],
      ['L', 'toon.network'],
    ],
  });

/** A Takeover of workload `id` from `from`, signed by `key`, `ago` seconds old. */
const takeover = (key: Uint8Array, id: string, from: Uint8Array, ago = 0) =>
  sign(key, {
    kind: 30433,
    created_at: now() - ago,
    content: JSON.stringify({ primary: getPublicKey(from) }),
    tags: [['d', id]],
  });

/** GET `path` from the relay under test, which must answer 200 with HTML. */
async function html(path: string): Promise<string> {
  const response = await fetch(`${relay.readUrl}${path}`);
  expect(response.status, path).toBe(200);
  return response.text();
}

/** The value of the first `data-state` in `text`. */
const state = (text: string) => /data-state="([^"]*)"/.exec(text)?.[1];

beforeAll(async () => {
  relay = await startRelay(imageUnderTest(), {
    env: { TOON_UI_PUBLIC_URL: ORIGIN },
  });
  for (const key of [sleepy, primary, standby, other]) {
    await publishOk(
      relay,
      sign(key, {
        kind: 10432,
        content: JSON.stringify({ name: 'Tidal' }),
        tags: [['L', 'toon.network']],
      })
    );
    await publishOk(
      relay,
      sign(key, {
        kind: 30432,
        content: JSON.stringify({
          name: 'Small',
          resources: { vcpu: 2 },
          price: 5,
          lease_interval_s: 60,
        }),
        tags: [
          ['d', 'small'],
          ['a', `10432:${getPublicKey(key)}:`],
          ['L', 'toon.network'],
        ],
      })
    );
  }
  for (const key of [primary, standby, other]) {
    await publishOk(
      relay,
      sign(key, {
        kind: 10433,
        content: '{}',
        tags: [
          ['expiration', String(now() + 3600)],
          ['L', 'toon.network'],
        ],
      })
    );
  }
  await publishOk(
    relay,
    sign(owner, {
      kind: 30617,
      created_at: now() - 100,
      tags: [
        ['d', 'site'],
        ['maintainers', getPublicKey(maintainer)],
      ],
    })
  );

  // One Deployment per state, each with its own workload id.
  await publishOk(relay, deployment(owner, 'down', sleepy, workload(1)));
  await publishOk(relay, deployment(owner, 'prod', primary, workload(2)));
  await publishOk(relay, deployment(owner, 'moved', primary, workload(3)));
  await publishOk(relay, deployment(owner, 'ended', primary, workload(4)));
  await publishOk(
    relay,
    deployment(maintainer, 'staging', primary, workload(5))
  );
  await publishOk(relay, deployment(stranger, 'fake', primary, workload(6)));
  await publishOk(relay, deployment(owner, 'elsewhere', other, workload(7)));

  // Keys `prod` does not name, claiming to end it and to take it over.
  await publishOk(relay, eviction(stranger, workload(2), { reason: 'abuse' }));
  await publishOk(relay, takeover(stranger, workload(2), primary));
  // The named standby, but taking over from a primary `prod` does not name.
  await publishOk(relay, takeover(standby, workload(2), other));
  // The standby takes over `moved`; the primary evicts `ended`.
  await publishOk(relay, takeover(standby, workload(3), primary, 200));
  await publishOk(
    relay,
    eviction(
      primary,
      workload(4),
      { reason: 'abuse', message: 'mining <b>' },
      300
    )
  );
  await publishOk(
    relay,
    eviction(
      primary,
      'ef'.repeat(32),
      { reason: 'made-up', message: 'too loud' },
      100
    )
  );
  // Another Provider's events, for its own log alone.
  await publishOk(
    relay,
    eviction(other, '12'.repeat(32), { reason: 'abuse', message: 'not here' })
  );
});

afterAll(async () => {
  await relay?.stop();
});

describe('relay image conformance: Deployments on /compute', () => {
  conformanceTest(
    'a Deployment page links its repository, commit, Provider, Listing, image and URL',
    async () => {
      const page = await html(`/compute/d/${npub(owner)}/prod`);
      for (const shown of [
        `href="/repo/${npub(owner)}/site"`,
        `href="/repo/${npub(owner)}/site/commit/${COMMIT}"`,
        `href="/compute/p/${npub(primary)}"`,
        `href="/compute/${npub(primary)}/small"`,
        `sha256:${IMAGE}`,
        'href="https://app.example.com"',
      ]) {
        expect(page).toContain(shown);
      }
      expect(page).not.toContain('data-claim');
    }
  );

  conformanceTest(
    'a Deployment whose primary has no live Liveness says provider down',
    async () => {
      expect(await html(`/compute/d/${npub(owner)}/down`)).toContain(
        'provider down'
      );
    }
  );

  conformanceTest(
    'a Deployment on a live primary is running until its expiration, whatever unnamed keys sign',
    async () => {
      const page = await html(`/compute/d/${npub(owner)}/prod`);
      expect(page).toContain('running on a live provider until');
      expect(page).not.toContain('evicted by');
      expect(page).not.toContain('taken over by');
    }
  );

  conformanceTest(
    'a Takeover by a standby the Deployment names says taken over, naming the winner',
    async () => {
      const page = await html(`/compute/d/${npub(owner)}/moved`);
      // The state itself names the winner, not only the Standby row.
      const at = page.indexOf('<span data-state="taken-over">');
      expect(at).toBeGreaterThan(-1);
      const shown = page.slice(at, page.indexOf('</a>', at));
      expect(shown).toContain('taken over by');
      expect(shown).toContain(`href="/compute/p/${npub(standby)}"`);
    }
  );

  conformanceTest(
    'an Eviction Notice from the primary says evicted, with its reason and message',
    async () => {
      const page = await html(`/compute/d/${npub(owner)}/ended`);
      expect(page).toContain('evicted by');
      expect(page).toContain('abuse');
      expect(page).toContain('mining &lt;b&gt;');
      expect(page).not.toContain('mining <b>');
    }
  );

  conformanceTest(
    "a stranger's Deployment of the repository has its own page, as their claim",
    async () => {
      expect(await html(`/compute/d/${npub(stranger)}/fake`)).toContain(
        'data-claim'
      );
      expect(
        await html(`/compute/d/${npub(maintainer)}/staging`)
      ).not.toContain('data-claim');
    }
  );

  conformanceTest("a Deployment's naddr redirects to its page", async () => {
    const naddr = nip19.naddrEncode({
      kind: 30437,
      pubkey: getPublicKey(owner),
      identifier: 'prod',
    });
    const response = await fetch(`${relay.readUrl}/compute/${naddr}`, {
      redirect: 'manual',
    });
    expect(response.status).toBeGreaterThanOrEqual(300);
    expect(response.status).toBeLessThan(400);
    expect(response.headers.get('location')).toBe(
      `/compute/d/${npub(owner)}/prod`
    );
  });

  conformanceTest(
    'a misshapen, unknown or expired Deployment is the 404 page',
    async () => {
      // An unmarked `x` breaks the shape.
      await publishOk(
        relay,
        deployment(owner, 'broken', primary, workload(8), {
          extra: [['x', 'ef'.repeat(32)]],
        })
      );
      await publishOk(
        relay,
        deployment(owner, 'expiring', primary, workload(9), {
          expiration: now() + 2,
        })
      );
      const expiring = `/compute/d/${npub(owner)}/expiring`;
      expect((await fetch(`${relay.readUrl}${expiring}`)).status).toBe(200);
      await new Promise((resolve) => setTimeout(resolve, 3_000));
      for (const path of [
        expiring,
        `/compute/d/${npub(owner)}/broken`,
        `/compute/d/${npub(owner)}/nothing`,
      ]) {
        const response = await fetch(`${relay.readUrl}${path}`);
        expect(response.status, path).toBe(404);
        expect(response.headers.get('content-security-policy'), path).toBe(
          POLICY
        );
        expect(await response.text(), path).toContain('Not found');
      }
    }
  );

  conformanceTest(
    'a Provider page lists what it serves, and logs its own Evictions, Takeovers and Deployments newest first',
    async () => {
      const page = await html(`/compute/p/${npub(primary)}`);
      expect(page).toContain('Serving');
      expect(page).toContain(`href="/compute/d/${npub(owner)}/prod"`);
      const log = page.slice(page.indexOf('data-activity>'));
      for (const shown of [
        'made-up',
        'too loud',
        'took over from',
        `/compute/p/${npub(standby)}`,
        `/compute/d/${npub(owner)}/prod`,
      ]) {
        expect(log).toContain(shown);
      }
      expect(log).not.toContain('not here');
      expect(log).not.toContain(`/compute/d/${npub(owner)}/elsewhere`);
      const times = [...log.matchAll(/data-at="(\d+)"/g)].map((m) =>
        Number(m[1])
      );
      expect(times.length).toBeGreaterThan(3);
      expect(times).toEqual([...times].sort((a, b) => b - a));

      const theirs = await html(`/compute/p/${npub(other)}`);
      const theirLog = theirs.slice(theirs.indexOf('data-activity>'));
      expect(theirLog).toContain('not here');
      expect(theirLog).not.toContain('too loud');
    }
  );

  conformanceTest(
    "the Environments tab lists the owner's and a maintainer's Deployments, not a stranger's",
    async () => {
      const tab = await html(`/repo/${npub(owner)}/site/environments`);
      for (const env of ['prod', 'staging', 'down', 'moved', 'ended']) {
        expect(tab, env).toContain(`data-environment="${env}"`);
      }
      expect(tab).not.toContain('data-environment="fake"');
      expect(tab).toContain('data-visit');
      const row = tab.split('data-environment="staging"')[1]?.split('</tr>')[0];
      expect(state(row ?? '')).toBeDefined();
      expect(state(row ?? '')).toBe(
        state(await html(`/compute/d/${npub(maintainer)}/staging`))
      );
      expect(await html(`/repo/${npub(owner)}/site`)).toContain(
        '/environments'
      );
    }
  );

  conformanceTest(
    'Provider, Listing and Deployment pages name their cards, which are 1200×630 PNGs with ETags',
    async () => {
      for (const [page, card] of [
        [`/compute/p/${npub(primary)}`, `/compute/og/p/${npub(primary)}.png`],
        [
          `/compute/${npub(primary)}/small`,
          `/compute/og/l/${npub(primary)}/small.png`,
        ],
        [
          `/compute/d/${npub(owner)}/prod`,
          `/compute/og/d/${npub(owner)}/prod.png`,
        ],
      ] as const) {
        const response = await fetch(`${relay.readUrl}${page}`);
        expect(response.headers.get('content-security-policy'), page).toBe(
          POLICY
        );
        const text = await response.text();
        for (const name of ['og:image', 'twitter:image']) {
          expect(text, `${page} ${name}`).toContain(
            `="${name}" content="${ORIGIN}${card}"`
          );
        }
        const image = await fetch(`${relay.readUrl}${card}`);
        expect(image.status, card).toBe(200);
        expect(image.headers.get('content-type')).toBe('image/png');
        const bytes = new Uint8Array(await image.arrayBuffer());
        const view = new DataView(bytes.buffer);
        expect([view.getUint32(16), view.getUint32(20)], card).toEqual([
          1200, 630,
        ]);
        const etag = image.headers.get('etag') ?? '';
        expect(etag).not.toBe('');
        const again = await fetch(`${relay.readUrl}${card}`, {
          headers: { 'if-none-match': etag },
        });
        expect(again.status, card).toBe(304);
      }
    }
  );
});
