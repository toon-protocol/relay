import { afterAll, beforeAll, describe, expect } from 'vitest';
import {
  Client,
  generateSecretKey,
  publish,
  signed,
  type Frame,
} from './harness/client.js';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { getDocument, waitForEdge } from './harness/nip11.js';
import { conformanceTest, imageUnderTest } from './implementation.js';

/** Kinds the `restricted` relay asks a client to authenticate to read. */
const PRIVATE_KINDS = [4, 1059];

let plain: RunningRelay;
let open: RunningRelay;
let restricted: RunningRelay;

beforeAll(async () => {
  plain = await startRelay(imageUnderTest());
  open = await startRelay(imageUnderTest(), {
    env: { TOON_NIP42_AUTH: 'true' },
  });
  restricted = await startRelay(imageUnderTest(), {
    env: { TOON_AUTH_REQUIRED_KINDS: PRIVATE_KINDS.join(',') },
  });
});

afterAll(async () => {
  await plain?.stop();
  await open?.stop();
  await restricted?.stop();
});

const isAuth = (frame: Frame): boolean => frame[0] === 'AUTH';

/** The challenge the relay sends a connection that has just opened. */
async function challengeOf(client: Client): Promise<string> {
  const frame = await client.next(isAuth);
  expect(typeof frame[1]).toBe('string');
  return frame[1] as string;
}

/** A kind-22242 event answering `challenge`, signed with `secretKey`. */
function answer(
  relay: RunningRelay,
  secretKey: Uint8Array,
  challenge: string,
  overrides: { created_at?: number; kind?: number } = {}
) {
  return signed(secretKey, {
    kind: 22242,
    tags: [
      ['relay', relay.readWsUrl],
      ['challenge', challenge],
    ],
    content: '',
    ...overrides,
  });
}

async function withClient(
  relay: RunningRelay,
  body: (client: Client) => Promise<void>
): Promise<void> {
  const client = await Client.connect(relay.readWsUrl);
  try {
    await body(client);
  } finally {
    client.close();
  }
}

describe('NIP-42: off by default', () => {
  conformanceTest('a relay with nothing set sends no challenge', async () => {
    await withClient(plain, async (client) => {
      expect(
        (await client.quiet()).filter(isAuth),
        'no AUTH challenge'
      ).toEqual([]);
    });
  });

  conformanceTest(
    'a relay with nothing set serves any kind to a client that has not authenticated',
    async () => {
      const event = signed(generateSecretKey(), { kind: 4 });
      await publish(plain.writeUrl, event);
      await withClient(plain, async (client) => {
        const found = await client.req('dm', { kinds: [4] });
        expect(found.map((e) => e.id)).toContain(event.id);
      });
    }
  );

  conformanceTest(
    'a relay with nothing set does not list 42 in supported_nips',
    async () => {
      await waitForEdge(plain.readUrl);
      const { body } = await getDocument(plain.readUrl);
      expect(body['supported_nips']).not.toContain(42);
    }
  );
});

describe('NIP-42: enabled', () => {
  conformanceTest(
    'lists 42 in supported_nips and still does not require auth of everyone',
    async () => {
      await waitForEdge(open.readUrl);
      const { body } = await getDocument(open.readUrl);
      expect(body['supported_nips']).toContain(42);
      expect(
        (body['limitation'] as unknown as { auth_required: boolean })
          .auth_required
      ).toBe(false);
    },
    { expectedFailureFor: ['typescript'] }
  );

  conformanceTest(
    'a new connection is sent a challenge, and each connection a different one',
    async () => {
      await withClient(open, async (first) => {
        await withClient(open, async (second) => {
          const [a, b] = [await challengeOf(first), await challengeOf(second)];
          expect(a).not.toBe('');
          expect(a).not.toBe(b);
        });
      });
    },
    { expectedFailureFor: ['typescript'] }
  );

  conformanceTest(
    'an AUTH that answers the challenge is accepted with OK true',
    async () => {
      await withClient(open, async (client) => {
        const auth = answer(
          open,
          generateSecretKey(),
          await challengeOf(client)
        );
        client.send(['AUTH', auth]);
        const ok = await client.next((f) => f[0] === 'OK' && f[1] === auth.id);
        expect(ok[2]).toBe(true);
      });
    },
    { expectedFailureFor: ['typescript'] }
  );

  conformanceTest(
    'an AUTH that answers another challenge, the wrong kind or a stale time is refused',
    async () => {
      await withClient(open, async (client) => {
        const challenge = await challengeOf(client);
        const key = generateSecretKey();
        const now = Math.floor(Date.now() / 1000);
        for (const auth of [
          answer(open, key, 'a-challenge-this-relay-never-issued'),
          answer(open, key, challenge, { kind: 1 }),
          answer(open, key, challenge, { created_at: now - 3600 }),
        ]) {
          client.send(['AUTH', auth]);
          const ok = await client.next(
            (f) => f[0] === 'OK' && f[1] === auth.id
          );
          expect(ok[2], JSON.stringify(auth)).toBe(false);
        }
      });
    },
    { expectedFailureFor: ['typescript'] }
  );

  conformanceTest(
    'an AUTH whose signature does not verify is refused',
    async () => {
      await withClient(open, async (client) => {
        const auth = answer(
          open,
          generateSecretKey(),
          await challengeOf(client)
        );
        const forged = { ...auth, sig: '0'.repeat(128) };
        client.send(['AUTH', forged]);
        const ok = await client.next((f) => f[0] === 'OK' && f[1] === auth.id);
        expect(ok[2]).toBe(false);
      });
    },
    { expectedFailureFor: ['typescript'] }
  );

  conformanceTest(
    'a challenge is good for the connection it was sent on and no other',
    async () => {
      await withClient(open, async (first) => {
        await withClient(open, async (second) => {
          const stolen = await challengeOf(first);
          await challengeOf(second);
          const auth = answer(open, generateSecretKey(), stolen);
          second.send(['AUTH', auth]);
          const ok = await second.next(
            (f) => f[0] === 'OK' && f[1] === auth.id
          );
          expect(ok[2]).toBe(false);
        });
      });
    },
    { expectedFailureFor: ['typescript'] }
  );
});

describe('NIP-42: required for chosen kinds', () => {
  const secret = generateSecretKey();

  conformanceTest(
    'the relay lists 42 and sends a challenge when kinds are chosen',
    async () => {
      await waitForEdge(restricted.readUrl);
      const { body } = await getDocument(restricted.readUrl);
      expect(body['supported_nips']).toContain(42);
      await withClient(restricted, async (client) => {
        await challengeOf(client);
      });
    },
    { expectedFailureFor: ['typescript'] }
  );

  conformanceTest(
    'a REQ that can return a chosen kind is closed auth-required before authenticating',
    async () => {
      await withClient(restricted, async (client) => {
        await challengeOf(client);
        for (const [id, filter] of [
          ['named', { kinds: [4] }],
          ['mixed', { kinds: [1, 1059] }],
          ['unnamed', {}],
        ] as const) {
          client.send(['REQ', id, filter]);
          const closed = await client.next(
            (f) => f[0] === 'CLOSED' && f[1] === id
          );
          expect(String(closed[2])).toMatch(/^auth-required:/);
        }
        expect(client.frames.some((f) => f[0] === 'EOSE')).toBe(false);
      });
    },
    { expectedFailureFor: ['typescript'] }
  );

  conformanceTest(
    'a REQ that cannot return a chosen kind is answered without authenticating',
    async () => {
      await withClient(restricted, async (client) => {
        await challengeOf(client);
        expect(await client.req('notes', { kinds: [1] })).toEqual(
          expect.any(Array)
        );
      });
    },
    { expectedFailureFor: ['typescript'] }
  );

  conformanceTest(
    'after a valid AUTH the same REQ is answered, and so are later ones',
    async () => {
      const event = signed(secret, { kind: 4 });
      await publish(restricted.writeUrl, event);
      await withClient(restricted, async (client) => {
        const challenge = await challengeOf(client);
        client.send(['REQ', 'before', { kinds: [4] }]);
        await client.next((f) => f[0] === 'CLOSED' && f[1] === 'before');

        const auth = answer(restricted, generateSecretKey(), challenge);
        client.send(['AUTH', auth]);
        const ok = await client.next((f) => f[0] === 'OK' && f[1] === auth.id);
        expect(ok[2]).toBe(true);

        const found = await client.req('after', { kinds: [4] });
        expect(found.map((e) => e.id)).toContain(event.id);
        expect((await client.req('all', {})).map((e) => e.id)).toContain(
          event.id
        );
      });
    },
    { expectedFailureFor: ['typescript'] }
  );
});
