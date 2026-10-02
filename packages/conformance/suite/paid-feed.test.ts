import { generateSecretKey } from 'nostr-tools/pure';
import { afterEach, describe, expect } from 'vitest';
import {
  removeVolume,
  runRelayToExit,
  startRelay,
  type RunningRelay,
  type StartOptions,
} from './harness/relay-container.js';
import {
  getDocument,
  settledDocument,
  waitForDocument,
} from './harness/nip11.js';
import { ilpDocument, STUB_ILP_ADDRESS } from './harness/stub-connector.js';
import { Client } from './harness/client.js';
import { publishOk, pubkeyOf, settle, sign } from './harness/wire.js';
import {
  authMessage,
  balanceOf,
  BROADCAST_PRICE,
  connectAndChallenge,
  connectAs,
  framesFor,
  nip98,
  pay,
  readBalance,
  sellingDocument,
  SUBSCRIBE_ADDRESS,
  SUBSCRIBE_PRICE,
  startSelling,
} from './harness/paid-feed.js';
import { conformanceTest, imageUnderTest } from './implementation.js';

// The paid live feed (#215, the draft NIP toon_cli#43).

const running: RunningRelay[] = [];
const clients: Client[] = [];

afterEach(async () => {
  for (const client of clients.splice(0)) client.close();
  await Promise.all(running.splice(0).map((relay) => relay.stop()));
});

async function selling(options: StartOptions = {}): Promise<RunningRelay> {
  const relay = await startSelling(imageUnderTest(), options);
  running.push(relay);
  return relay;
}

async function connectedAs(
  relay: RunningRelay,
  secretKey: Uint8Array
): Promise<Client> {
  const client = await connectAs(relay, secretKey);
  clients.push(client);
  return client;
}

const kind1 = (author: Uint8Array, content = 'live') =>
  sign(author, { kind: 1, content });

const eventIds = (frames: unknown[][]): string[] =>
  frames
    .filter((f) => f[0] === 'EVENT')
    .map((f) => (f[2] as { id: string }).id);

describe('relay image conformance: the subscribe route', () => {
  conformanceTest(
    'a first payment with a filter opens a subscription and answers it',
    async () => {
      const relay = await selling();
      const key = generateSecretKey();
      const filter = { kinds: [1] };
      const answer = await pay(relay, key, { body: { filter } });
      expect(answer.status).toBe(200);
      expect(answer.body).toEqual({
        pubkey: pubkeyOf(key),
        credited: SUBSCRIBE_PRICE,
        balance: SUBSCRIBE_PRICE,
        broadcast_price: BROADCAST_PRICE,
        filter,
      });
    }
  );

  conformanceTest(
    'a later payment tops up, and {} keeps the filter',
    async () => {
      const relay = await selling();
      const key = generateSecretKey();
      const filter = { kinds: [1], '#t': ['a'] };
      await pay(relay, key, { body: { filter } });
      const second = await pay(relay, key, { body: {} });
      expect(second.status).toBe(200);
      expect(second.body).toMatchObject({
        credited: SUBSCRIBE_PRICE,
        balance: 2 * SUBSCRIBE_PRICE,
        filter,
      });
    }
  );

  conformanceTest('a later packet may replace the filter', async () => {
    const relay = await selling();
    const key = generateSecretKey();
    await pay(relay, key, { body: { filter: { kinds: [1] } } });
    const second = await pay(relay, key, {
      body: { filter: { kinds: [7] } },
    });
    expect(second.body['filter']).toEqual({ kinds: [7] });
  });

  conformanceTest(
    'what the connector states it charged is what is credited, whoever paid',
    async () => {
      const relay = await selling();
      const key = generateSecretKey();
      const first = await pay(relay, key, {
        body: { filter: {} },
        amount: '2500',
        payer: `solana:${'1'.repeat(32)}`,
      });
      expect(first.body).toMatchObject({ credited: 2500, balance: 2500 });
      // Another payer, the same key.
      const second = await pay(relay, key, {
        amount: '500',
        payer: `evm:0x${'cd'.repeat(32)}`,
      });
      expect(second.body).toMatchObject({ credited: 500, balance: 3000 });
    }
  );

  conformanceTest(
    'a packet that states no amount credits the route price the connector publishes',
    async () => {
      const relay = await selling();
      const key = generateSecretKey();
      const answer = await pay(relay, key, {
        body: { filter: {} },
        amount: null,
      });
      expect(answer.status).toBe(200);
      expect(answer.body).toMatchObject({ credited: SUBSCRIBE_PRICE });
    }
  );

  conformanceTest(
    '401 unauthorized for an authorization that is missing or does not hold',
    async () => {
      const relay = await selling();
      const key = generateSecretKey();
      const body = JSON.stringify({ filter: {} });
      const stale = nip98(key, {
        body,
        createdAt: Math.floor(Date.now() / 1000) - 120,
      });
      const cases: Record<string, string | null> = {
        missing: null,
        'not Nostr': 'Bearer abc',
        'not an event': 'Nostr bm90IGFuIGV2ZW50',
        'wrong method': nip98(key, { body, method: 'GET' }),
        'another relay': nip98(key, { body, url: 'https://elsewhere.test/' }),
        'another body': nip98(key, { body: '{"filter":{"kinds":[2]}}' }),
        'no payload': nip98(key),
        stale,
      };
      for (const [name, authorization] of Object.entries(cases)) {
        const answer = await pay(relay, key, { body, authorization });
        expect(answer.status, name).toBe(401);
        expect(answer.body, name).toEqual({
          error: { code: 'unauthorized', message: expect.any(String) },
        });
      }
      // None of them credited anything.
      const read = await readBalance(relay, nip98(key, { method: 'GET' }));
      expect(read.status).toBe(404);
    }
  );

  conformanceTest(
    '400 invalid_request for a body that is not an object or a filter that is not one',
    async () => {
      const relay = await selling();
      const key = generateSecretKey();
      for (const body of [
        '[]',
        '"x"',
        'not json',
        '{"filter":[]}',
        '{"filter":{"kinds":"x"}}',
      ]) {
        const answer = await pay(relay, key, { body });
        expect(answer.status, body).toBe(400);
        expect(answer.body['error'], body).toMatchObject({
          code: 'invalid_request',
        });
      }
    }
  );

  conformanceTest(
    '400 filter_required for a first payment without a filter, and nothing is credited',
    async () => {
      const relay = await selling();
      const key = generateSecretKey();
      const answer = await pay(relay, key, { body: {} });
      expect(answer.status).toBe(400);
      expect(answer.body['error']).toMatchObject({ code: 'filter_required' });
      expect(
        (await readBalance(relay, nip98(key, { method: 'GET' }))).status
      ).toBe(404);
    }
  );

  conformanceTest('a refused top-up credits nothing', async () => {
    const relay = await selling();
    const key = generateSecretKey();
    await pay(relay, key, { body: { filter: {} } });
    const refused = await pay(relay, key, { body: { filter: [] } });
    expect(refused.status).toBe(400);
    expect(await balanceOf(relay, key)).toBe(SUBSCRIBE_PRICE);
  });
});

describe('relay image conformance: reading the balance', () => {
  conformanceTest(
    'the subscriber reads its balance, its price and its filter',
    async () => {
      const relay = await selling();
      const key = generateSecretKey();
      const filter = { kinds: [1] };
      await pay(relay, key, { body: { filter } });
      const read = await readBalance(relay, nip98(key, { method: 'GET' }));
      expect(read.status).toBe(200);
      expect(read.contentType).toContain('application/toon-subscription+json');
      expect(read.body).toEqual({
        pubkey: pubkeyOf(key),
        balance: SUBSCRIBE_PRICE,
        broadcast_price: BROADCAST_PRICE,
        filter,
      });
    }
  );

  conformanceTest(
    '404 not_subscribed for a key with no subscription; 401 for no authorization',
    async () => {
      const relay = await selling();
      const key = generateSecretKey();
      const none = await readBalance(relay, nip98(key, { method: 'GET' }));
      expect(none.status).toBe(404);
      expect(none.body['error']).toMatchObject({ code: 'not_subscribed' });
      const unauthorized = await readBalance(relay, null);
      expect(unauthorized.status).toBe(401);
      expect(unauthorized.body['error']).toMatchObject({
        code: 'unauthorized',
      });
      const posted = await readBalance(
        relay,
        nip98(key, { method: 'POST', body: '' })
      );
      expect(posted.status).toBe(401);
    }
  );

  conformanceTest(
    'the operator lists subscribers and balances on the write port',
    async () => {
      const relay = await selling();
      const [one, two] = [generateSecretKey(), generateSecretKey()];
      await pay(relay, one, { body: { filter: { kinds: [1] } } });
      await pay(relay, two, { body: { filter: { kinds: [7] } }, amount: '40' });
      const response = await fetch(`${relay.writeUrl}/subscribers`);
      expect(response.status).toBe(200);
      const body = (await response.json()) as {
        subscribers: { pubkey: string; balance: number }[];
      };
      const balances = Object.fromEntries(
        body.subscribers.map((s) => [s.pubkey, s.balance])
      );
      expect(balances).toEqual({
        [pubkeyOf(one)]: SUBSCRIBE_PRICE,
        [pubkeyOf(two)]: 40,
      });
    }
  );

  conformanceTest(
    'a balance survives a restart over the same volume',
    async () => {
      const volume = `conformance-paid-feed-${process.pid}-${Date.now()}`;
      try {
        const key = generateSecretKey();
        const first = await startSelling(imageUnderTest(), { volume });
        try {
          await pay(first, key, { body: { filter: { kinds: [1] } } });
        } finally {
          await first.stop();
        }
        const second = await selling({ volume });
        expect(await balanceOf(second, key)).toBe(SUBSCRIBE_PRICE);
      } finally {
        await removeVolume(volume);
      }
    }
  );
});

describe('relay image conformance: the information document', () => {
  conformanceTest(
    'publishes toon_subscription from the connector and the settings, and lists NIP-42',
    async () => {
      const relay = await selling();
      const document = await waitForDocument(
        relay.readUrl,
        (body) => body['toon_subscription'] !== undefined
      );
      expect(document['toon_subscription']).toEqual({
        ilp_address: SUBSCRIBE_ADDRESS,
        price: SUBSCRIBE_PRICE,
        broadcast_price: BROADCAST_PRICE,
      });
      expect(document.supported_nips).toContain(42);
      expect(document['toon']).toBeDefined();
    }
  );

  conformanceTest(
    "carries the subscribe route's carriage when the connector pins one",
    async () => {
      const relay = await selling({
        document: sellingDocument({ requiredTransport: 'btp' }),
      });
      const document = await waitForDocument(
        relay.readUrl,
        (body) => body['toon_subscription'] !== undefined
      );
      expect(document['toon_subscription']).toMatchObject({ carriage: 'btp' });
    }
  );

  conformanceTest(
    'publishes nothing while the connector does not terminate the address',
    async () => {
      const relay = await selling({
        document: ilpDocument({
          routes: [{ prefix: STUB_ILP_ADDRESS, price: '1000' }],
        }),
      });
      const document = await settledDocument(
        relay.readUrl,
        relay.connector.requests
      );
      expect(document['toon']).toBeDefined();
      expect(document['toon_subscription']).toBeUndefined();
    }
  );

  conformanceTest(
    'publishes nothing for a route that is not a flat price above zero',
    async () => {
      for (const route of [{ price: '0' }, { pricePerKib: '10' }]) {
        const relay = await selling({ document: sellingDocument(route) });
        const document = await settledDocument(
          relay.readUrl,
          relay.connector.requests
        );
        expect(
          document['toon_subscription'],
          JSON.stringify(route)
        ).toBeUndefined();
        await relay.stop();
        running.splice(running.indexOf(relay), 1);
      }
    }
  );

  conformanceTest(
    'a relay that sells nothing publishes no toon_subscription and keeps its feed free',
    async () => {
      const relay = await startRelay(imageUnderTest(), {
        document: sellingDocument(),
      });
      running.push(relay);
      const { body } = await getDocument(relay.readUrl);
      expect(body['toon_subscription']).toBeUndefined();
      expect(body.supported_nips).not.toContain(42);
    }
  );
});

describe('relay image conformance: the live feed', () => {
  conformanceTest(
    'the relay challenges every connection with NIP-42',
    async () => {
      const relay = await selling();
      const { client, challenge } = await connectAndChallenge(relay);
      clients.push(client);
      expect(challenge).not.toBe('');
      const other = await connectAndChallenge(relay);
      clients.push(other.client);
      expect(other.challenge).not.toBe(challenge);
    }
  );

  conformanceTest(
    'an AUTH with another challenge, another relay or a wrong kind is refused',
    async () => {
      const relay = await selling();
      const key = generateSecretKey();
      const { client, challenge } = await connectAndChallenge(relay);
      clients.push(client);
      for (const message of [
        authMessage(key, 'not-the-challenge'),
        authMessage(key, challenge, 'wss://elsewhere.test'),
      ]) {
        client.send(message);
        const ok = await client.next(
          (f) => f[0] === 'OK' && f[1] === message[1].id
        );
        expect(ok[2]).toBe(false);
      }
    }
  );

  conformanceTest(
    'a subscriber is sent the live events its filter asks for and each is debited',
    async () => {
      const relay = await selling();
      const key = generateSecretKey();
      const author = generateSecretKey();
      await pay(relay, key, { body: { filter: { kinds: [1] } } });
      const client = await connectedAs(relay, key);
      const stored = kind1(author, 'stored');
      await publishOk(relay, stored);

      client.send(['REQ', 'feed', { kinds: [1] }]);
      await client.next((f) => f[0] === 'EOSE' && f[1] === 'feed');
      expect(eventIds(await framesFor(client, 'feed', 0))).toEqual([]);
      // The stored event came before EOSE and was not debited.
      expect(await balanceOf(relay, key)).toBe(SUBSCRIBE_PRICE);

      const live = kind1(author, 'live');
      await publishOk(relay, live);
      const frame = await client.next(
        (f) => f[0] === 'EVENT' && f[1] === 'feed'
      );
      expect((frame[2] as { id: string }).id).toBe(live.id);

      // Not the subscription's filter: not sent, not debited.
      await publishOk(relay, sign(author, { kind: 7, content: '+' }));
      expect(eventIds(await framesFor(client, 'feed'))).toEqual([]);
      expect(await balanceOf(relay, key)).toBe(
        SUBSCRIBE_PRICE - BROADCAST_PRICE
      );
    }
  );

  conformanceTest(
    'an event the subscription pays for but no open REQ asks for is not debited',
    async () => {
      const relay = await selling();
      const key = generateSecretKey();
      const author = generateSecretKey();
      await pay(relay, key, { body: { filter: { kinds: [1, 7] } } });
      const client = await connectedAs(relay, key);
      client.send(['REQ', 'feed', { kinds: [1] }]);
      await client.next((f) => f[0] === 'EOSE');
      await publishOk(relay, sign(author, { kind: 7, content: '+' }));
      expect(eventIds(await framesFor(client, 'feed'))).toEqual([]);
      expect(await balanceOf(relay, key)).toBe(SUBSCRIBE_PRICE);
    }
  );

  conformanceTest(
    'an event costs one broadcast price however many connections and REQs of the subscriber get it',
    async () => {
      const relay = await selling();
      const key = generateSecretKey();
      const author = generateSecretKey();
      await pay(relay, key, { body: { filter: { kinds: [1] } } });
      const a = await connectedAs(relay, key);
      const b = await connectedAs(relay, key);
      for (const [client, id] of [
        [a, 'one'],
        [a, 'two'],
        [b, 'three'],
      ] as const) {
        client.send(['REQ', id, { kinds: [1] }]);
        await client.next((f) => f[0] === 'EOSE' && f[1] === id);
      }
      const live = kind1(author);
      await publishOk(relay, live);
      // One connection's subscriptions hear an event in no set order.
      const first = await a.next(
        (f) => f[0] === 'EVENT' && (f[1] === 'one' || f[1] === 'two')
      );
      const second = first[1] === 'one' ? 'two' : 'one';
      await a.next((f) => f[0] === 'EVENT' && f[1] === second);
      await b.next((f) => f[0] === 'EVENT' && f[1] === 'three');
      expect(await balanceOf(relay, key)).toBe(
        SUBSCRIBE_PRICE - BROADCAST_PRICE
      );
    }
  );

  conformanceTest(
    'when the balance reaches zero the feed is closed with a payment-required message, and the connection stays',
    async () => {
      const relay = await selling();
      const key = generateSecretKey();
      const author = generateSecretKey();
      // Paid for two events.
      await pay(relay, key, {
        body: { filter: { kinds: [1] } },
        amount: String(2 * BROADCAST_PRICE),
      });
      const client = await connectedAs(relay, key);
      client.send(['REQ', 'feed', { kinds: [1] }]);
      await client.next((f) => f[0] === 'EOSE');

      const events = [
        kind1(author, '1'),
        kind1(author, '2'),
        kind1(author, '3'),
      ];
      for (const event of events) await publishOk(relay, event);

      await client.next((f) => f[0] === 'EVENT' && f[1] === 'feed');
      await client.next((f) => f[0] === 'EVENT' && f[1] === 'feed');
      const closed = await client.next(
        (f) => f[0] === 'CLOSED' && f[1] === 'feed'
      );
      expect(String(closed[2])).toMatch(/^payment-required:/);
      // The third is not sent, and the relay left the connection open.
      expect(eventIds(await framesFor(client, 'feed'))).toEqual([]);
      expect(client.closed).toBeUndefined();
      expect(await balanceOf(relay, key)).toBe(0);

      // A new REQ is a free read: stored events, EOSE, then payment-required.
      client.send(['REQ', 'again', { kinds: [1] }]);
      await client.next((f) => f[0] === 'EOSE' && f[1] === 'again');
      const refused = await client.next(
        (f) => f[0] === 'CLOSED' && f[1] === 'again'
      );
      expect(String(refused[2])).toMatch(/^payment-required:/);
    }
  );

  conformanceTest(
    'the balance left over counts toward the next payment',
    async () => {
      const relay = await selling();
      const key = generateSecretKey();
      await pay(relay, key, {
        body: { filter: {} },
        amount: String(BROADCAST_PRICE - 1),
      });
      // Less than one broadcast price: exhausted, and its remainder stays.
      const client = await connectedAs(relay, key);
      client.send(['REQ', 'feed', {}]);
      await client.next((f) => f[0] === 'EOSE');
      const closed = await client.next((f) => f[0] === 'CLOSED');
      expect(String(closed[2])).toMatch(/^payment-required:/);
      const top = await pay(relay, key, { amount: '5' });
      expect(top.body).toMatchObject({ balance: BROADCAST_PRICE + 4 });
    }
  );

  conformanceTest(
    'a free read returns stored events, EOSE, then CLOSED with auth-required, and no live events',
    async () => {
      const relay = await selling();
      const author = generateSecretKey();
      const stored = kind1(author, 'stored');
      await publishOk(relay, stored);
      const client = await Client.connect(relay.readWsUrl);
      clients.push(client);
      client.send(['REQ', 'q', { kinds: [1] }]);
      const frames: string[] = [];
      for (;;) {
        const frame = await client.next((f) => f[1] === 'q');
        frames.push(String(frame[0]));
        if (frame[0] === 'CLOSED') {
          expect(String(frame[2])).toMatch(/^auth-required:/);
          break;
        }
      }
      expect(frames).toEqual(['EVENT', 'EOSE', 'CLOSED']);
      await publishOk(relay, kind1(author, 'after'));
      expect(await framesFor(client, 'q')).toEqual([]);
    }
  );

  conformanceTest(
    'an authenticated connection with no subscription is closed with payment-required',
    async () => {
      const relay = await selling();
      const client = await connectedAs(relay, generateSecretKey());
      client.send(['REQ', 'q', { kinds: [1] }]);
      await client.next((f) => f[0] === 'EOSE' && f[1] === 'q');
      const closed = await client.next(
        (f) => f[0] === 'CLOSED' && f[1] === 'q'
      );
      expect(String(closed[2])).toMatch(/^payment-required:/);
    }
  );

  conformanceTest(
    'a payment moves the feed to the key that signed it, not to whoever paid',
    async () => {
      const relay = await selling();
      const [paid, other] = [generateSecretKey(), generateSecretKey()];
      const author = generateSecretKey();
      await pay(relay, paid, { body: { filter: { kinds: [1] } } });
      const stranger = await connectedAs(relay, other);
      stranger.send(['REQ', 'feed', { kinds: [1] }]);
      await stranger.next((f) => f[0] === 'EOSE');
      const closed = await stranger.next((f) => f[0] === 'CLOSED');
      expect(String(closed[2])).toMatch(/^payment-required:/);
      await publishOk(relay, kind1(author));
      await settle(500);
      expect(await balanceOf(relay, paid)).toBe(SUBSCRIBE_PRICE);
    }
  );

  conformanceTest(
    "the relay's own key follows the live feed without a subscription and without a debit",
    async () => {
      const relay = await selling();
      const author = generateSecretKey();
      const operator = Uint8Array.from(Buffer.from(relay.secretKey, 'hex'));
      const client = await connectedAs(relay, operator);
      client.send(['REQ', 'feed', { kinds: [1] }]);
      await client.next((f) => f[0] === 'EOSE');
      const live = kind1(author);
      await publishOk(relay, live);
      const frame = await client.next(
        (f) => f[0] === 'EVENT' && f[1] === 'feed'
      );
      expect((frame[2] as { id: string }).id).toBe(live.id);
      expect(
        (await readBalance(relay, nip98(operator, { method: 'GET' }))).status
      ).toBe(404);
    }
  );

  conformanceTest(
    'a key the operator names follows the live feed too',
    async () => {
      const named = generateSecretKey();
      const relay = await selling({
        env: { TOON_OPERATOR_PUBKEYS: pubkeyOf(named) },
      });
      const client = await connectedAs(relay, named);
      client.send(['REQ', 'feed', { kinds: [1] }]);
      await client.next((f) => f[0] === 'EOSE');
      const live = kind1(generateSecretKey());
      await publishOk(relay, live);
      await client.next((f) => f[0] === 'EVENT' && f[1] === 'feed');
    }
  );
});

describe('relay image conformance: settings of the paid feed', () => {
  conformanceTest(
    'the settings are named by documented variables and the route needs the connector',
    async () => {
      const base = { TOON_SECRET_KEY: '1'.repeat(64) };
      for (const env of [
        { TOON_BROADCAST_PRICE: '10' },
        { TOON_SUBSCRIBE_ILP_ADDRESS: SUBSCRIBE_ADDRESS },
        {
          TOON_SUBSCRIBE_ILP_ADDRESS: SUBSCRIBE_ADDRESS,
          TOON_BROADCAST_PRICE: '0',
          TOON_RELAY_URL: 'wss://relay.conformance.test',
          TOON_CONNECTOR_URL: 'http://connector:3000/ilp',
          TOON_WRITE_ILP_ADDRESS: 'g.toon.relay',
        },
        {
          TOON_SUBSCRIBE_ILP_ADDRESS: SUBSCRIBE_ADDRESS,
          TOON_BROADCAST_PRICE: '10',
          TOON_RELAY_URL: 'not a url',
          TOON_CONNECTOR_URL: 'http://connector:3000/ilp',
          TOON_WRITE_ILP_ADDRESS: 'g.toon.relay',
        },
        {
          TOON_SUBSCRIBE_ILP_ADDRESS: SUBSCRIBE_ADDRESS,
          TOON_BROADCAST_PRICE: '10',
          TOON_RELAY_URL: 'wss://relay.conformance.test',
        },
      ]) {
        const exited = await runRelayToExit(imageUnderTest(), {
          env: { ...base, ...env },
        });
        expect(exited.code, JSON.stringify(env)).not.toBe(0);
        expect(exited.output, JSON.stringify(env)).toMatch(/Error:/);
      }
    }
  );
});
