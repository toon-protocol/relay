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

  const discarded: [string, Record<string, string>][] = [
    ['absent', {}],
    ['missing the payer', { ...TRIPLE, 'X-TOON-Payer': '' }],
    ['missing the amount', { ...TRIPLE, 'X-TOON-Amount': '' }],
    ['missing the chain', { ...TRIPLE, 'X-TOON-Chain': '' }],
    ['an unknown chain', { ...TRIPLE, 'X-TOON-Chain': 'bitcoin' }],
    ['a non-numeric amount', { ...TRIPLE, 'X-TOON-Amount': '10.5' }],
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
      try {
        const event = signedEvent(7777);
        expect((await write({ event })).status).toBe(200);
        const delivered = await subscription.next(10_000);
        expect(delivered?.id).toBe(event.id);
      } finally {
        subscription.close();
      }
      expect((await query(relay.readWsUrl, { kinds: [7777] })).length).toBe(1);
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
    const event = signedEvent(20100, 'x'.repeat(16 * 1024));
    expect((await ephemeral({ event })).status).toBe(413);
  });
});
