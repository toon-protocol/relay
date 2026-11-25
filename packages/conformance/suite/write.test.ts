import { afterAll, beforeAll, describe, expect } from 'vitest';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { post, query, signedEvent, subscribe } from './harness/client.js';
import { conformanceTest, imageUnderTest } from './implementation.js';

let relay: RunningRelay;

beforeAll(async () => {
  relay = await startRelay(imageUnderTest());
});

afterAll(async () => {
  await relay?.stop();
});

const EVM_PAYER = `evm:0x${'ab'.repeat(32)}`;
const SOLANA_PAYER = 'solana:' + '1'.repeat(32);
const TRIPLE = {
  'X-TOON-Payer': EVM_PAYER,
  'X-TOON-Amount': '1000',
  'X-TOON-Chain': 'evm',
};

/** TRIPLE with one header left out entirely. */
const without = (name: keyof typeof TRIPLE): Record<string, string> =>
  Object.fromEntries(Object.entries(TRIPLE).filter(([key]) => key !== name));

const write = (body: unknown, headers: Record<string, string> = {}) =>
  post(`${relay.writeUrl}/write`, body, headers);

describe('relay image conformance: POST /write', () => {
  conformanceTest('200 carries the event id and a stored-at time', async () => {
    const event = signedEvent(1);
    const response = await write({ event });
    expect(response.status).toBe(200);
    expect(await response.json()).toEqual({
      eventId: event.id,
      storedAt: expect.any(Number),
    });
  });

  conformanceTest('400 for a body that is not JSON', async () => {
    expect((await write('not json')).status).toBe(400);
  });

  conformanceTest('400 for a body with no event', async () => {
    expect((await write({})).status).toBe(400);
  });

  conformanceTest('422 for a bad signature', async () => {
    const event = signedEvent(1);
    const bad = { ...event, sig: '0'.repeat(128) };
    expect((await write({ event: bad })).status).toBe(422);
  });

  conformanceTest('422 for an id that does not match the content', async () => {
    const event = signedEvent(1);
    const bad = { ...event, content: 'tampered after signing' };
    expect((await write({ event: bad })).status).toBe(422);
  });

  conformanceTest(
    '422 with "Invalid event id" for an id that is not the hash of the event',
    async () => {
      const event = signedEvent(1);
      const other = event.id.startsWith('f') ? '0' : 'f';
      const bad = { ...event, id: other.repeat(64) };
      const response = await write({ event: bad });
      expect(response.status).toBe(422);
      expect(await response.json()).toEqual({ error: 'Invalid event id' });
    }
  );

  conformanceTest(
    'GET /write is 405 with an Allow header naming POST',
    async () => {
      const response = await fetch(`${relay.writeUrl}/write`);
      expect(response.status).toBe(405);
      expect(response.headers.get('allow') ?? '').toMatch(/\bPOST\b/);
    }
  );

  conformanceTest(
    'a retired path returns 404: /publish and /handle-packet',
    async () => {
      const event = signedEvent(1);
      for (const path of ['/publish', '/handle-packet']) {
        const response = await post(`${relay.writeUrl}${path}`, { event });
        expect(response.status, path).toBe(404);
      }
    }
  );
});

describe('relay image conformance: payment attribution', () => {
  conformanceTest(
    'a complete, well-formed, consistent EVM triple is echoed',
    async () => {
      const response = await write({ event: signedEvent(1) }, TRIPLE);
      expect(response.status).toBe(200);
      expect(await response.json()).toMatchObject({
        payment: { payer: EVM_PAYER, amount: '1000', chain: 'evm' },
      });
    }
  );

  conformanceTest('a Solana triple is echoed', async () => {
    const response = await write(
      { event: signedEvent(1) },
      {
        'X-TOON-Payer': SOLANA_PAYER,
        'X-TOON-Amount': '5',
        'X-TOON-Chain': 'solana',
      }
    );
    expect(response.status).toBe(200);
    expect(await response.json()).toMatchObject({
      payment: { payer: SOLANA_PAYER, amount: '5', chain: 'solana' },
    });
  });

  const kept: [string, Record<string, string>, Record<string, string>][] = [
    [
      'an XRPL statement with its network',
      {
        'X-TOON-Payer': 'xrpl:rN7n7otQDd6FczFgLdSqtcsAUxDkw6fzRH',
        'X-TOON-Amount': '12',
        'X-TOON-Chain': 'xrpl',
        'X-TOON-Network': 'xrpl:testnet',
      },
      {
        payer: 'xrpl:rN7n7otQDd6FczFgLdSqtcsAUxDkw6fzRH',
        amount: '12',
        chain: 'xrpl',
        network: 'xrpl:testnet',
      },
    ],
    [
      'a zk allowance statement with its claim key',
      {
        'X-TOON-Payer': 'zk-note:42',
        'X-TOON-Amount': '3',
        'X-TOON-Chain': 'mina',
        'X-TOON-Network': 'mina:devnet',
      },
      {
        payer: 'zk-note:42',
        amount: '3',
        chain: 'mina',
        network: 'mina:devnet',
      },
    ],
    [
      'a statement on a chain the relay has not heard of',
      {
        'X-TOON-Payer': 'bitcoin:bc1qexample',
        'X-TOON-Amount': '1',
        'X-TOON-Chain': 'bitcoin',
      },
      { payer: 'bitcoin:bc1qexample', amount: '1', chain: 'bitcoin' },
    ],
  ];
  for (const [name, headers, payment] of kept) {
    conformanceTest(`${name} is echoed as stated`, async () => {
      const response = await write({ event: signedEvent(1) }, headers);
      expect(response.status).toBe(200);
      expect(await response.json()).toMatchObject({ payment });
    });
  }

  const discarded: [string, Record<string, string>][] = [
    ['absent', {}],
    ['missing the payer', without('X-TOON-Payer')],
    ['missing the amount', without('X-TOON-Amount')],
    ['missing the chain', without('X-TOON-Chain')],
    ['carrying an empty payer', { ...TRIPLE, 'X-TOON-Payer': '' }],
    [
      'a chain that is not a namespace',
      { ...TRIPLE, 'X-TOON-Chain': 'bit coin' },
    ],
    ['a non-integer amount', { ...TRIPLE, 'X-TOON-Amount': '10.5' }],
    ['a malformed payer', { ...TRIPLE, 'X-TOON-Payer': 'evm:0x1234' }],
    ['a payer of the wrong chain', { ...TRIPLE, 'X-TOON-Chain': 'solana' }],
  ];
  for (const [name, headers] of discarded) {
    conformanceTest(
      `a triple that is ${name} is discarded whole and the write succeeds`,
      async () => {
        const response = await write({ event: signedEvent(1) }, headers);
        expect(response.status).toBe(200);
        expect(await response.json()).not.toHaveProperty('payment');
      }
    );
  }
});

describe('relay image conformance: live delivery of stored writes', () => {
  conformanceTest(
    'a stored write reaches a live subscriber without a re-query',
    async () => {
      const subscription = await subscribe(relay.readWsUrl, { kinds: [7777] });
      const event = signedEvent(7777);
      try {
        expect((await write({ event })).status).toBe(200);
        const delivered = await subscription.next(10_000);
        expect(delivered?.id).toBe(event.id);
      } finally {
        subscription.close();
      }
      const stored = await query(relay.readWsUrl, { ids: [event.id] });
      expect(stored.map((e) => e.id)).toEqual([event.id]);
    }
  );
});

describe('relay image conformance: POST /write-ephemeral', () => {
  const ephemeral = (body: unknown) =>
    post(`${relay.writeUrl}/write-ephemeral`, body);

  conformanceTest(
    'verifies, fans out to a live subscriber and does not store',
    async () => {
      const subscription = await subscribe(relay.readWsUrl, {
        kinds: [20100],
      });
      try {
        const event = signedEvent(20100);
        const response = await ephemeral({ event });
        expect(response.status).toBe(200);
        expect(await response.json()).toEqual({
          eventId: event.id,
          broadcastAt: expect.any(Number),
        });
        expect((await subscription.next(10_000))?.id).toBe(event.id);
        expect(await query(relay.readWsUrl, { ids: [event.id] })).toEqual([]);
      } finally {
        subscription.close();
      }
    }
  );

  conformanceTest('400 for a kind outside the ephemeral range', async () => {
    expect((await ephemeral({ event: signedEvent(1) })).status).toBe(400);
  });

  conformanceTest('400 for a body with no event', async () => {
    expect((await ephemeral({})).status).toBe(400);
  });

  conformanceTest('422 for a bad signature', async () => {
    const event = signedEvent(20100);
    const bad = { ...event, sig: '0'.repeat(128) };
    expect((await ephemeral({ event: bad })).status).toBe(422);
  });

  conformanceTest('413 for a body over the size cap', async () => {
    // Twice the relay's default 8 KiB ephemeral body cap.
    const event = signedEvent(20100, 'x'.repeat(16 * 1024));
    expect((await ephemeral({ event })).status).toBe(413);
  });
});
