import { afterAll, afterEach, beforeAll, describe, expect } from 'vitest';
import { getPublicKey } from 'nostr-tools/pure';
import {
  Client,
  generateSecretKey,
  publish,
  signed,
  sleep,
  type Frame,
} from './harness/client.js';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { getDocument, waitForEdge } from './harness/nip11.js';
import { startSelling, connectAs } from './harness/paid-feed.js';
import { publishOk } from './harness/wire.js';
import { conformanceTest, imageUnderTest } from './implementation.js';

// TOON_NIP17_RECIPIENT_ONLY (#264): a kind 1059 gift wrap is read only by a
// connection that proved, under NIP-42, a key its `p` tags name.

const WRAP = 1059;
const AUTH_REQUIRED = /^auth-required:/;

let relay: RunningRelay;
let both: RunningRelay;
let plain: RunningRelay;
const extra: RunningRelay[] = [];
const clients: Client[] = [];

beforeAll(async () => {
  relay = await startRelay(imageUnderTest(), {
    env: { TOON_NIP17_RECIPIENT_ONLY: 'true' },
  });
  both = await startRelay(imageUnderTest(), {
    env: {
      TOON_NIP17_RECIPIENT_ONLY: 'true',
      TOON_AUTH_REQUIRED_KINDS: String(WRAP),
    },
  });
  plain = await startRelay(imageUnderTest());
});

afterAll(async () => {
  await relay?.stop();
  await both?.stop();
  await plain?.stop();
});

afterEach(async () => {
  for (const client of clients.splice(0)) client.close();
  await Promise.all(extra.splice(0).map((r) => r.stop()));
});

const pub = (key: Uint8Array): string => getPublicKey(key);

/** A wrap from a throwaway key addressed to each of `recipients`. */
let counter = 0;
function wrap(recipients: Uint8Array[], createdAt?: number) {
  return signed(generateSecretKey(), {
    kind: WRAP,
    tags: recipients.map((r) => ['p', pub(r)]),
    content: `wrap ${counter++}`,
    ...(createdAt === undefined ? {} : { created_at: createdAt }),
  });
}

async function connect(target: RunningRelay): Promise<Client> {
  const client = await Client.connect(target.readWsUrl);
  clients.push(client);
  return client;
}

async function authenticate(
  target: RunningRelay,
  client: Client,
  key: Uint8Array
): Promise<void> {
  const challenge = (await client.next((f) => f[0] === 'AUTH'))[1] as string;
  const auth = signed(key, {
    kind: 22242,
    tags: [
      ['relay', target.readWsUrl],
      ['challenge', challenge],
    ],
    content: '',
  });
  client.send(['AUTH', auth]);
  const ok = await client.next((f) => f[0] === 'OK' && f[1] === auth.id);
  expect(ok[2]).toBe(true);
}

async function connectedAs(
  target: RunningRelay,
  key: Uint8Array
): Promise<Client> {
  const client = await connect(target);
  await authenticate(target, client, key);
  return client;
}

const ids = (events: { id: string }[]): string[] => events.map((e) => e.id);

describe('TOON_NIP17_RECIPIENT_ONLY: gift wraps are read by their recipients', () => {
  conformanceTest(
    'lists 42 in supported_nips and challenges every connection',
    async () => {
      await waitForEdge(relay.readUrl);
      const { body } = await getDocument(relay.readUrl);
      expect(body['supported_nips']).toContain(42);
      const client = await connect(relay);
      await client.next((f) => f[0] === 'AUTH');
    }
  );

  conformanceTest(
    'a key the wrap does not name gets none of it, and the recipient gets it, from kinds, {} and ids',
    async () => {
      const recipient = generateSecretKey();
      const event = wrap([recipient]);
      await publish(relay.writeUrl, event);
      const other = await connectedAs(relay, generateSecretKey());
      for (const filter of [{ kinds: [WRAP] }, {}, { ids: [event.id] }]) {
        expect(
          ids(await other.req('o', filter)),
          JSON.stringify(filter)
        ).not.toContain(event.id);
      }
      const reader = await connectedAs(relay, recipient);
      for (const filter of [{ kinds: [WRAP] }, {}, { ids: [event.id] }]) {
        expect(
          ids(await reader.req('r', filter)),
          JSON.stringify(filter)
        ).toContain(event.id);
      }
    }
  );

  conformanceTest(
    'a wrap with several p tags is served to a key that proved any one of them',
    async () => {
      const [a, b, c] = [
        generateSecretKey(),
        generateSecretKey(),
        generateSecretKey(),
      ];
      const event = wrap([a, b]);
      await publish(relay.writeUrl, event);
      for (const key of [a, b]) {
        const client = await connectedAs(relay, key);
        expect(ids(await client.req('s', { kinds: [WRAP] }))).toContain(
          event.id
        );
      }
      const stranger = await connectedAs(relay, c);
      expect(ids(await stranger.req('s', { kinds: [WRAP] }))).not.toContain(
        event.id
      );
    }
  );

  conformanceTest(
    'an unauthenticated REQ naming 1059 is closed auth-required; {} is answered without wraps',
    async () => {
      const event = wrap([generateSecretKey()]);
      const note = signed(generateSecretKey(), { kind: 1 });
      await publish(relay.writeUrl, event);
      await publish(relay.writeUrl, note);
      const client = await connect(relay);
      await client.next((f) => f[0] === 'AUTH');
      for (const [id, filter] of [
        ['named', { kinds: [WRAP] }],
        ['mixed', { kinds: [1, WRAP] }],
      ] as const) {
        client.send(['REQ', id, filter]);
        const closed = await client.next(
          (f) => f[0] === 'CLOSED' && f[1] === id
        );
        expect(String(closed[2])).toMatch(AUTH_REQUIRED);
      }
      const found = await client.req('all', {});
      expect(ids(found)).toContain(note.id);
      expect(ids(found)).not.toContain(event.id);
    }
  );

  conformanceTest(
    'an authenticated key asking for wraps addressed to someone else gets none',
    async () => {
      const someone = generateSecretKey();
      const event = wrap([someone]);
      await publish(relay.writeUrl, event);
      const client = await connectedAs(relay, generateSecretKey());
      expect(
        await client.req('p', { kinds: [WRAP], '#p': [pub(someone)] })
      ).toEqual([]);
    }
  );

  conformanceTest(
    'an open subscription is sent a wrap stored after EOSE only if the connection may read it',
    async () => {
      const recipient = generateSecretKey();
      const reader = await connectedAs(relay, recipient);
      const other = await connectedAs(relay, generateSecretKey());
      await reader.req('live', { kinds: [WRAP] });
      await other.req('live', { kinds: [WRAP] });
      const mine = wrap([recipient]);
      const theirs = wrap([generateSecretKey()]);
      await publish(relay.writeUrl, theirs);
      await publish(relay.writeUrl, mine);
      const got = await reader.next((f) => f[0] === 'EVENT' && f[1] === 'live');
      expect((got[2] as { id: string }).id).toBe(mine.id);
      const seenByOther = (await other.quiet()).filter(
        (f: Frame) => f[0] === 'EVENT'
      );
      expect(seenByOther).toEqual([]);
      const seenByReader = (await reader.quiet()).filter(
        (f: Frame) => f[0] === 'EVENT'
      );
      expect(seenByReader).toEqual([]);
    }
  );

  conformanceTest(
    "other keys' newer wraps do not use up the reader's limit",
    async () => {
      const recipient = generateSecretKey();
      const now = Math.floor(Date.now() / 1000);
      const mine = wrap([recipient], now - 100);
      await publish(relay.writeUrl, mine);
      for (let i = 0; i < 3; i++) {
        await publish(
          relay.writeUrl,
          wrap([generateSecretKey()], now - 10 + i)
        );
      }
      const client = await connectedAs(relay, recipient);
      const found = await client.req('lim', { kinds: [WRAP], limit: 2 });
      expect(ids(found)).toEqual([mine.id]);
    }
  );

  conformanceTest(
    "the relay's own key and a named operator read only the wraps addressed to them",
    async () => {
      const named = generateSecretKey();
      const operators = await startRelay(imageUnderTest(), {
        env: {
          TOON_NIP17_RECIPIENT_ONLY: 'true',
          TOON_OPERATOR_PUBKEYS: pub(named),
        },
      });
      extra.push(operators);
      const event = wrap([generateSecretKey()]);
      const addressed = wrap([named]);
      await publish(operators.writeUrl, event);
      await publish(operators.writeUrl, addressed);
      const ownKey = Uint8Array.from(Buffer.from(operators.secretKey, 'hex'));
      for (const key of [ownKey, named]) {
        const client = await connectedAs(operators, key);
        const found = ids(await client.req('op', { kinds: [WRAP] }));
        expect(found).not.toContain(event.id);
        expect(found.includes(addressed.id)).toBe(key === named);
      }
    }
  );

  conformanceTest(
    'on a relay that sells its feed an operator following it live still sees only its own wraps',
    async () => {
      const named = generateSecretKey();
      const selling = await startSelling(imageUnderTest(), {
        env: {
          TOON_NIP17_RECIPIENT_ONLY: 'true',
          TOON_OPERATOR_PUBKEYS: pub(named),
        },
      });
      extra.push(selling);
      const own = Uint8Array.from(Buffer.from(selling.secretKey, 'hex'));
      for (const key of [own, named]) {
        const client = await connectAs(selling, key);
        clients.push(client);
        client.send(['REQ', 'feed', { kinds: [WRAP] }]);
        await client.next((f) => f[0] === 'EOSE');
        const theirs = wrap([generateSecretKey()]);
        const mine = wrap([key]);
        await publishOk(selling, theirs);
        await publishOk(selling, mine);
        const got = await client.next(
          (f) => f[0] === 'EVENT' && f[1] === 'feed'
        );
        expect((got[2] as { id: string }).id).toBe(mine.id);
        await sleep(300);
      }
    }
  );

  conformanceTest(
    'when TOON_AUTH_REQUIRED_KINDS also names 1059 a REQ naming no kinds is closed auth-required',
    async () => {
      const client = await connect(both);
      await client.next((f) => f[0] === 'AUTH');
      client.send(['REQ', 'all', {}]);
      const closed = await client.next(
        (f) => f[0] === 'CLOSED' && f[1] === 'all'
      );
      expect(String(closed[2])).toMatch(AUTH_REQUIRED);
    }
  );

  conformanceTest(
    'with the setting off a wrap is served to any connection, and none is challenged',
    async () => {
      const event = wrap([generateSecretKey()]);
      await publish(plain.writeUrl, event);
      const client = await connect(plain);
      expect(ids(await client.req('w', { kinds: [WRAP] }))).toContain(event.id);
      expect((await client.quiet(300)).some((f) => f[0] === 'AUTH')).toBe(
        false
      );
    }
  );
});
