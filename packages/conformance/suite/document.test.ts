import { afterEach, describe, expect } from 'vitest';
import {
  startRelay,
  type Env,
  type RunningRelay,
  type StartOptions,
} from './harness/relay-container.js';
import { getDocument, settledDocument, waitForEdge } from './harness/nip11.js';
import {
  ilpDocument,
  STUB_ILP_ADDRESS,
  STUB_SEAL_KEY,
  STUB_SETTLEMENTS,
  STUB_WRITE_EDGE,
} from './harness/stub-connector.js';
import { conformanceTest, imageUnderTest } from './implementation.js';

const running: RunningRelay[] = [];

afterEach(async () => {
  await Promise.all(running.splice(0).map((relay) => relay.stop()));
});

async function relayWith(options: StartOptions): Promise<RunningRelay> {
  const relay = await startRelay(imageUnderTest(), options);
  running.push(relay);
  return relay;
}

/** The document's carriage once the edge is known (`undefined` if unpinned). */
async function carriageOf(options: StartOptions): Promise<unknown> {
  const relay = await relayWith(options);
  const document = await waitForEdge(relay.readUrl);
  return document.toon?.carriage;
}

const route = (extra: Record<string, unknown> = {}) => [
  { prefix: STUB_ILP_ADDRESS, price: '1000', ...extra },
];

describe('relay image conformance: the information document edge', () => {
  conformanceTest(
    'a known edge puts ilp_address, connector_url, seal key, carriage, price and settlement under toon',
    async () => {
      const relay = await relayWith({
        document: ilpDocument({ routes: route({ requiredTransport: 'http' }) }),
      });
      const document = await waitForEdge(relay.readUrl);
      expect(document['toon']).toEqual({
        ilp_address: STUB_ILP_ADDRESS,
        connector_url: STUB_WRITE_EDGE,
        connector_seal_key: STUB_SEAL_KEY,
        carriage: 'http',
        price: 1000,
        settlement: STUB_SETTLEMENTS,
      });
    }
  );

  conformanceTest(
    'no connector configured: the document is served without a toon object',
    async () => {
      const relay = await relayWith({ connector: 'none' });
      const document = await settledDocument(relay.readUrl);
      expect(document['pubkey']).toMatch(/^[0-9a-f]{64}$/);
      expect(document).not.toHaveProperty('toon');
    }
  );

  conformanceTest(
    'connector unreachable: the document is served without a toon object',
    async () => {
      const relay = await relayWith({ connector: 'down' });
      const document = await settledDocument(relay.readUrl);
      expect(document['pubkey']).toMatch(/^[0-9a-f]{64}$/);
      expect(document).not.toHaveProperty('toon');
    }
  );

  conformanceTest(
    'a configured address the connector does not terminate: the document is served without a toon object',
    async () => {
      const relay = await relayWith({
        env: { TOON_WRITE_ILP_ADDRESS: 'g.toon.elsewhere' },
      });
      const document = await settledDocument(relay.readUrl, () =>
        relay.connector.requests()
      );
      expect(document['pubkey']).toMatch(/^[0-9a-f]{64}$/);
      expect(document).not.toHaveProperty('toon');
    }
  );

  conformanceTest('carriage: the route wins over the node', async () => {
    expect(
      await carriageOf({
        document: ilpDocument({
          routes: route({ requiredTransport: 'http' }),
          requiredTransport: 'btp',
        }),
        env: { TOON_WRITE_CARRIAGE: 'btp' },
      })
    ).toBe('http');
  });

  conformanceTest(
    'carriage: a route that pins nothing falls to the node, which wins over the operator setting',
    async () => {
      expect(
        await carriageOf({
          document: ilpDocument({
            routes: route({ requiredTransport: 'both' }),
            requiredTransport: 'btp',
          }),
          env: { TOON_WRITE_CARRIAGE: 'http' },
        })
      ).toBe('btp');
    }
  );

  conformanceTest(
    'carriage: with neither route nor node pinning one, the operator setting is used',
    async () => {
      expect(
        await carriageOf({
          document: ilpDocument({ requiredTransport: 'both' }),
          env: { TOON_WRITE_CARRIAGE: 'btp' },
        })
      ).toBe('btp');
    }
  );

  conformanceTest(
    'carriage: with nothing pinning one, the document states none',
    async () => {
      expect(await carriageOf({})).toBeUndefined();
    }
  );
});

describe('relay image conformance: the information document body', () => {
  let relay: RunningRelay;

  async function paidDocument(env?: Env) {
    relay = await relayWith({ ...(env !== undefined && { env }) });
    await waitForEdge(relay.readUrl);
    return getDocument(relay.readUrl);
  }

  conformanceTest(
    'a paid relay states limitation, fees.publication, supported_nips, software and version',
    async () => {
      const { status, headers, body } = await paidDocument();
      expect(status).toBe(200);
      expect(headers.get('content-type')).toContain('application/nostr+json');
      expect(body['limitation']).toEqual({
        payment_required: true,
        restricted_writes: true,
        max_subscriptions: expect.any(Number),
        max_filters: expect.any(Number),
        auth_required: false,
      });
      expect(body['fees']).toEqual({
        publication: [{ amount: 1000, unit: 'uusdc' }],
      });
      expect(body['supported_nips']).toEqual([1, 9, 11, 16, 40]);
      expect(body['software']).toEqual(expect.any(String));
      expect(body['software']).not.toBe('');
      expect(body['version']).toEqual(expect.any(String));
      expect(body['version']).not.toBe('');
      const health = (await (
        await fetch(`${relay.writeUrl}/health`)
      ).json()) as { pubkey: string };
      expect(body['pubkey']).toBe(health.pubkey);
    }
  );

  conformanceTest(
    'a price of zero omits fees and says payment is not required, but still names the edge',
    async () => {
      relay = await relayWith({
        document: ilpDocument({
          routes: [{ prefix: STUB_ILP_ADDRESS, price: '0' }],
        }),
      });
      const document = await waitForEdge(relay.readUrl);
      expect(document).not.toHaveProperty('fees');
      expect(document.limitation?.payment_required).toBe(false);
      expect(document.limitation?.restricted_writes).toBe(true);
      expect(document.toon?.price).toBe(0);
    }
  );

  conformanceTest(
    'supported_nips drops 40 when expiration is not enforced',
    async () => {
      const { body } = await paidDocument({ TOON_ENFORCE_EXPIRATION: 'false' });
      expect(body['supported_nips']).toEqual([1, 9, 11, 16]);
    }
  );

  conformanceTest(
    'the document is cross-origin readable and OPTIONS answers 204',
    async () => {
      const { headers } = await paidDocument();
      expect(headers.get('access-control-allow-origin')).toBe('*');

      const preflight = await fetch(relay.readUrl, { method: 'OPTIONS' });
      expect(preflight.status).toBe(204);
      expect(preflight.headers.get('access-control-allow-origin')).toBe('*');
      expect(preflight.headers.get('access-control-allow-methods')).toContain(
        'GET'
      );
      expect(preflight.headers.get('access-control-allow-headers')).toContain(
        'accept'
      );
    }
  );
});
