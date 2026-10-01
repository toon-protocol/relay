import { getPublicKey } from 'nostr-tools/pure';
import { afterAll, beforeAll, describe, expect } from 'vitest';
import {
  Client,
  generateSecretKey,
  publish,
  signed,
  sleep,
} from './harness/client.js';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import {
  ilpDocument,
  STUB_ILP_ADDRESS,
  STUB_PRICE,
  STUB_WRITE_EDGE,
} from './harness/stub-connector.js';
import { conformanceTest, imageUnderTest } from './implementation.js';

let relay: RunningRelay;
let capped: RunningRelay;

const MAX_CONNECTIONS = 3;
/** The carriage the stub pins on the relay's route, so a refusal can name it. */
const STUB_CARRIAGE = 'btp';
/** An id no test ever stores, for REQs that only need an EOSE. */
const NO_SUCH_ID = '0'.repeat(64);

beforeAll(async () => {
  relay = await startRelay(imageUnderTest(), {
    document: ilpDocument({
      routes: [
        {
          prefix: STUB_ILP_ADDRESS,
          price: STUB_PRICE,
          requiredTransport: STUB_CARRIAGE,
        },
      ],
    }),
  });
  capped = await startRelay(imageUnderTest(), {
    env: { TOON_MAX_CONNECTIONS: String(MAX_CONNECTIONS) },
  });
});

afterAll(async () => {
  await relay?.stop();
  await capped?.stop();
});

const ids = (events: { id: string }[]): string[] =>
  events.map((e) => e.id).sort();

/** The `limitation` the relay states in its NIP-11 document. */
async function limitation(): Promise<{
  max_subscriptions: number;
  max_filters: number;
}> {
  const response = await fetch(relay.readUrl, {
    headers: { accept: 'application/nostr+json' },
  });
  const document = (await response.json()) as {
    limitation: { max_subscriptions: number; max_filters: number };
  };
  return document.limitation;
}

/** Run `body` with a fresh connection that is always closed afterwards. */
async function withClient(body: (c: Client) => Promise<void>): Promise<void> {
  const client = await Client.connect(relay.readWsUrl);
  try {
    await body(client);
  } finally {
    client.close();
  }
}

describe('read side: filters', () => {
  conformanceTest(
    'kinds, since, until and authors each narrow a filter',
    async () => {
      const key = generateSecretKey();
      const other = generateSecretKey();
      const a = signed(key, { kind: 1, created_at: 1_700_000_100 });
      const b = signed(key, { kind: 1, created_at: 1_700_000_200 });
      const c = signed(key, { kind: 7, created_at: 1_700_000_300 });
      const d = signed(other, { kind: 1, created_at: 1_700_000_200 });
      for (const e of [a, b, c, d]) await publish(relay.writeUrl, e);
      const author = getPublicKey(key);

      await withClient(async (client) => {
        expect(
          ids(await client.req('k', { authors: [author], kinds: [1] }))
        ).toEqual(ids([a, b]));
        expect(
          ids(
            await client.req('s', { authors: [author], since: 1_700_000_200 })
          )
        ).toEqual(ids([b, c]));
        expect(
          ids(
            await client.req('u', { authors: [author], until: 1_700_000_200 })
          )
        ).toEqual(ids([a, b]));
        expect(
          ids(
            await client.req('w', {
              authors: [author],
              since: 1_700_000_150,
              until: 1_700_000_250,
            })
          )
        ).toEqual(ids([b]));
        expect(
          ids(
            await client.req('a', {
              authors: [author, getPublicKey(other)],
              kinds: [1],
              since: 1_700_000_200,
              until: 1_700_000_200,
            })
          )
        ).toEqual(ids([b, d]));
      });
    }
  );

  conformanceTest(
    'ids select events by id, and tag filters by tag value',
    async () => {
      const key = generateSecretKey();
      const alpha = signed(key, {
        tags: [['t', 'alpha']],
        created_at: 1_700_000_001,
      });
      const beta = signed(key, {
        tags: [['t', 'beta']],
        created_at: 1_700_000_002,
      });
      const gamma = signed(key, {
        tags: [['t', 'gamma']],
        created_at: 1_700_000_003,
      });
      for (const e of [alpha, beta, gamma]) await publish(relay.writeUrl, e);
      const author = getPublicKey(key);

      await withClient(async (client) => {
        expect(
          ids(await client.req('i', { ids: [alpha.id, gamma.id] }))
        ).toEqual(ids([alpha, gamma]));
        expect(
          ids(
            await client.req('t', {
              authors: [author],
              '#t': ['alpha', 'beta'],
            })
          )
        ).toEqual(ids([alpha, beta]));
        // Conditions AND: the id is `alpha`'s but the tag value is not.
        expect(
          await client.req('and', { ids: [alpha.id], '#t': ['gamma'] })
        ).toEqual([]);
      });
    }
  );

  conformanceTest(
    'filters in one REQ are OR-ed, conditions within a filter AND-ed',
    async () => {
      const key = generateSecretKey();
      const author = getPublicKey(key);
      const note = signed(key, { kind: 1, created_at: 1_700_001_001 });
      const reaction = signed(key, { kind: 7, created_at: 1_700_001_002 });
      const article = signed(key, {
        kind: 30023,
        tags: [['d', 'x']],
        created_at: 1_700_001_003,
      });
      for (const e of [note, reaction, article])
        await publish(relay.writeUrl, e);

      await withClient(async (client) => {
        expect(
          ids(
            await client.req(
              'or',
              { authors: [author], kinds: [1] },
              { authors: [author], kinds: [7] }
            )
          )
        ).toEqual(ids([note, reaction]));
        // AND within a filter: `note` is kind 1 but too old, `reaction` is
        // recent enough but kind 7, so only `article` meets every condition.
        expect(
          ids(
            await client.req('and', {
              authors: [author],
              kinds: [1, 30023],
              since: 1_700_001_002,
            })
          )
        ).toEqual(ids([article]));
      });
    }
  );

  conformanceTest(
    'limit applies per filter, not to the whole REQ',
    async () => {
      const key = generateSecretKey();
      const author = getPublicKey(key);
      const notes = [1, 2, 3].map((i) =>
        signed(key, { kind: 1, created_at: 1_700_002_000 + i })
      );
      const reactions = [1, 2, 3].map((i) =>
        signed(key, { kind: 7, created_at: 1_700_002_000 + i })
      );
      for (const e of [...notes, ...reactions])
        await publish(relay.writeUrl, e);

      await withClient(async (client) => {
        const found = await client.req(
          'lim',
          { authors: [author], kinds: [1], limit: 1 },
          { authors: [author], kinds: [7], limit: 2 }
        );
        // The newest note, and the two newest reactions.
        expect(ids(found)).toEqual(
          ids([...notes.slice(-1), ...reactions.slice(-2)])
        );
      });
    },
    { expectedFailureFor: ['typescript'] }
  );

  conformanceTest(
    'ids are matched exactly, not as prefixes',
    async () => {
      const event = signed(generateSecretKey(), { created_at: 1_700_003_000 });
      await publish(relay.writeUrl, event);

      await withClient(async (client) => {
        expect(ids(await client.req('full', { ids: [event.id] }))).toEqual([
          event.id,
        ]);
        expect(
          await client.req('idp', { ids: [event.id.slice(0, 16)] })
        ).toEqual([]);
      });
    },
    { expectedFailureFor: ['typescript'] }
  );

  conformanceTest(
    'authors are matched exactly, not as prefixes',
    async () => {
      const event = signed(generateSecretKey(), { created_at: 1_700_003_001 });
      await publish(relay.writeUrl, event);

      await withClient(async (client) => {
        expect(
          ids(await client.req('full', { authors: [event.pubkey] }))
        ).toEqual([event.id]);
        expect(
          await client.req('aup', { authors: [event.pubkey.slice(0, 16)] })
        ).toEqual([]);
      });
    },
    { expectedFailureFor: ['typescript'] }
  );

  conformanceTest('event frames are plain NIP-01 JSON objects', async () => {
    const event = signed(generateSecretKey(), {
      tags: [['t', 'frame']],
      content: 'héllo "quoted"',
    });
    await publish(relay.writeUrl, event);
    await withClient(async (client) => {
      client.send(['REQ', 'frame', { ids: [event.id] }]);
      const frame = await client.next((f) => f[0] === 'EVENT');
      // Parsed, not byte-for-byte: key order and whitespace are the relay's.
      expect(frame).toEqual([
        'EVENT',
        'frame',
        JSON.parse(JSON.stringify(event)),
      ]);
    });
  });
});

describe('read side: subscriptions', () => {
  conformanceTest(
    'a subscription stays live after EOSE and receives a later write',
    async () => {
      const key = generateSecretKey();
      const author = getPublicKey(key);
      await withClient(async (client) => {
        expect(await client.req('live', { authors: [author] })).toEqual([]);
        const event = signed(key);
        await publish(relay.writeUrl, event);
        const frame = await client.next(
          (f) => f[0] === 'EVENT' && f[1] === 'live'
        );
        expect((frame[2] as { id: string }).id).toBe(event.id);
      });
    }
  );

  conformanceTest('CLOSE stops a subscription', async () => {
    const key = generateSecretKey();
    const author = getPublicKey(key);
    await withClient(async (client) => {
      await client.req('gone', { authors: [author] });
      client.send(['CLOSE', 'gone']);
      // A second subscription proves CLOSE was processed before the write.
      await client.req('barrier', { authors: [author] });
      await publish(relay.writeUrl, signed(key));
      // The write was fanned out, so `gone` would have had it by now.
      await client.next((f) => f[0] === 'EVENT' && f[1] === 'barrier');
      const rest = await client.quiet();
      expect(rest.filter((f) => f[1] === 'gone')).toEqual([]);
    });
  });

  conformanceTest(
    're-using a subscription id replaces the subscription',
    async () => {
      const first = generateSecretKey();
      const second = generateSecretKey();
      await withClient(async (client) => {
        await client.req('same', { authors: [getPublicKey(first)] });
        await client.req('same', { authors: [getPublicKey(second)] });
        await publish(relay.writeUrl, signed(first));
        const replacement = signed(second);
        await publish(relay.writeUrl, replacement);
        const frame = await client.next(
          (f) => f[0] === 'EVENT' && f[1] === 'same'
        );
        expect((frame[2] as { id: string }).id).toBe(replacement.id);
        expect((await client.quiet()).filter((f) => f[0] === 'EVENT')).toEqual(
          []
        );
      });
    }
  );
});

describe('read side: EVENT over WebSocket', () => {
  conformanceTest(
    'is refused with OK false naming the write edge',
    async () => {
      const event = signed(generateSecretKey());
      // The relay reads the edge in the background; retry until it is named.
      const deadline = Date.now() + 20_000;
      let message = '';
      await withClient(async (client) => {
        do {
          client.send(['EVENT', event]);
          const frame = await client.next((f) => f[0] === 'OK');
          expect(frame.slice(0, 3)).toEqual(['OK', event.id, false]);
          message = String(frame[3]);
          if (message.includes(STUB_ILP_ADDRESS)) break;
          await sleep(250);
        } while (Date.now() < deadline);
      });
      expect(message).toContain(STUB_ILP_ADDRESS);
      expect(message).toContain(STUB_WRITE_EDGE);
      expect(message).toMatch(new RegExp(`\\b${STUB_CARRIAGE}\\b`));
      expect(message).toMatch(new RegExp(`\\b${STUB_PRICE}\\b`));

      // And it was never stored.
      await withClient(async (client) => {
        expect(await client.req('stored', { ids: [event.id] })).toEqual([]);
      });
    }
  );
});

describe('read side: malformed input', () => {
  const notices: [string, unknown][] = [
    ['bad JSON', '{not json'],
    ['a non-array message', '{"a":1}'],
    ['an unknown message type', ['BOGUS', 'x']],
    ['an invalid subscription id', ['REQ', '', {}]],
    ['a non-string subscription id', ['REQ', 7, {}]],
  ];
  for (const [name, message] of notices) {
    conformanceTest(`${name} gets a NOTICE`, async () => {
      await withClient(async (client) => {
        client.send(message);
        const frame = await client.next((f) => f[0] === 'NOTICE');
        expect(typeof frame[1]).toBe('string');
      });
    });
  }
});

describe('read side: limits', () => {
  conformanceTest('the subscription limit is enforced', async () => {
    const limit = (await limitation()).max_subscriptions;
    await withClient(async (client) => {
      for (let i = 0; i < limit; i++)
        await client.req(`s${i}`, { ids: [NO_SUCH_ID] });
      client.send(['REQ', 'one-too-many', { ids: [NO_SUCH_ID] }]);
      await client.next((f) => f[0] === 'NOTICE');
      // Give a relay that NOTICEs and serves anyway time to send its EOSE.
      await client.quiet();
      expect(
        client.frames.some((f) => f[0] === 'EOSE' && f[1] === 'one-too-many')
      ).toBe(false);
      // Replacing an existing subscription is not a new one.
      expect(await client.req('s0', { ids: [NO_SUCH_ID] })).toEqual([]);
    });
  });

  conformanceTest('the filter limit is enforced', async () => {
    const limit = (await limitation()).max_filters;
    await withClient(async (client) => {
      const filter = { ids: [NO_SUCH_ID] };
      expect(await client.req('ok', ...Array(limit).fill(filter))).toEqual([]);
      client.send(['REQ', 'many', ...Array(limit + 1).fill(filter)]);
      await client.next((f) => f[0] === 'NOTICE');
      await client.quiet();
      expect(
        client.frames.some((f) => f[0] === 'EOSE' && f[1] === 'many')
      ).toBe(false);
    });
  });

  conformanceTest(
    'the connection cap closes the excess connection with 1013',
    async () => {
      const held: Client[] = [];
      try {
        for (let i = 0; i < MAX_CONNECTIONS; i++) {
          const client = await Client.connect(capped.readWsUrl);
          held.push(client);
          // A REQ round trip proves the connection was admitted.
          await client.req('hold', { ids: [NO_SUCH_ID] });
        }
        const excess = await Client.connect(capped.readWsUrl);
        held.push(excess);
        expect((await excess.untilClosed()).code).toBe(1013);
      } finally {
        for (const client of held) client.close();
      }
    }
  );
});

describe('read side: plain HTTP on the read port', () => {
  conformanceTest(
    'GET without the NIP-11 accept header answers 426',
    async () => {
      const response = await fetch(relay.readUrl);
      expect(response.status).toBe(426);
    }
  );
});
