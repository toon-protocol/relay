import { afterEach, describe, expect } from 'vitest';
import {
  DEFAULT_SECRET_KEY,
  runRelayToExit,
  startRelay,
  type Env,
  type RunningRelay,
} from './harness/relay-container.js';
import { getDocument, waitForEdge } from './harness/nip11.js';
import {
  conformanceTest,
  imageUnderTest,
  type ConformanceTestOptions,
} from './implementation.js';
import { getPublicKey } from 'nostr-tools/pure';
import { STUB_ILP_ADDRESS, STUB_PRICE } from './harness/stub-connector.js';

const running: RunningRelay[] = [];

afterEach(async () => {
  await Promise.all(running.splice(0).map((relay) => relay.stop()));
});

describe('relay image conformance: a connector that is down at start', () => {
  conformanceTest(
    'the relay starts, serves no edge, and publishes it once the connector appears',
    async () => {
      const relay = await startRelay(imageUnderTest(), { connector: 'down' });
      running.push(relay);
      expect((await getDocument(relay.readUrl)).body).not.toHaveProperty(
        'toon'
      );

      await relay.connector.start();
      const document = await waitForEdge(relay.readUrl);
      expect(document['toon']).toMatchObject({
        ilp_address: STUB_ILP_ADDRESS,
        price: Number(STUB_PRICE),
      });
    },
    { expectedFailureFor: ['rust'] }
  );
});

describe('relay image conformance: settings the relay refuses to start with', () => {
  const key = DEFAULT_SECRET_KEY;
  // The Rust relay reads only its identity, its two listeners and its data
  // directory so far (#193), so it starts on every other setting.
  const notYetInRust: ConformanceTestOptions = {
    expectedFailureFor: ['rust'],
  };
  const refused: [
    string,
    { env?: Env; args?: string[] },
    ConformanceTestOptions?,
  ][] = [
    [
      'an invalid port',
      { env: { TOON_SECRET_KEY: key, TOON_RELAY_PORT: 'x' } },
    ],
    [
      'an invalid carriage',
      { env: { TOON_SECRET_KEY: key, TOON_WRITE_CARRIAGE: 'both' } },
      notYetInRust,
    ],
    ['an invalid secret key', { env: { TOON_SECRET_KEY: 'not-hex' } }],
    ['a missing identity', { env: {} }],
    [
      'a connector URL without a write address',
      {
        env: {
          TOON_SECRET_KEY: key,
          TOON_CONNECTOR_URL: 'http://connector.invalid/ilp',
        },
      },
      notYetInRust,
    ],
    [
      'a malformed blocklist id',
      { env: { TOON_SECRET_KEY: key, TOON_BLOCKED_EVENT_IDS: 'zz' } },
      notYetInRust,
    ],
    [
      'an unknown flag',
      { env: { TOON_SECRET_KEY: key }, args: ['--no-such-flag'] },
      notYetInRust,
    ],
  ];

  for (const [name, options, expectation] of refused) {
    conformanceTest(
      `${name} exits non-zero with an Error: line`,
      async () => {
        const exited = await runRelayToExit(imageUnderTest(), options);
        expect(exited.code).not.toBe(0);
        expect(exited.output).toMatch(/Error: /);
      },
      expectation
    );
  }
});

describe('relay image conformance: documented environment variables', () => {
  const blocked = ['aa'.repeat(32), 'bb'.repeat(32)].join(',');

  conformanceTest(
    'every documented variable is accepted, and takes effect where it is visible',
    async () => {
      const relay = await startRelay(imageUnderTest(), {
        env: {
          NOSTR_SECRET_KEY: '2'.repeat(64),
          TOON_SECRET_KEY: DEFAULT_SECRET_KEY,
          TOON_RELAY_PORT: '7200',
          TOON_BLS_PORT: '3200',
          TOON_HOST: '0.0.0.0',
          TOON_WRITE_HOST: '0.0.0.0',
          TOON_DATA_DIR: '/tmp/conformance-data',
          TOON_DEV_MODE: 'true',
          TOON_VERIFY_EPHEMERAL: 'true',
          TOON_VERIFY_WORKERS: '0',
          TOON_MAX_CONNECTIONS: '64',
          TOON_EPHEMERAL_RATE_LIMIT: '5',
          TOON_EPHEMERAL_RATE_WINDOW_MS: '2000',
          TOON_EPHEMERAL_MAX_BODY_BYTES: '4096',
          TOON_WRITE_CARRIAGE: 'btp',
          TOON_RELAY_NAME: 'conformance relay',
          TOON_RELAY_DESCRIPTION: 'a relay under test',
          TOON_RELAY_CONTACT: 'mailto:ops@example.invalid',
          TOON_LOG_WRITES: 'true',
          TOON_ENFORCE_EXPIRATION: 'false',
          TOON_EXPIRATION_REAP_GRACE_SECONDS: '60',
          TOON_EXPIRATION_REAP_INTERVAL_SECONDS: '0',
          TOON_BLOCKED_EVENT_IDS: blocked,
        },
      });
      running.push(relay);

      const document = await waitForEdge(relay.readUrl);
      expect(document).toMatchObject({
        name: 'conformance relay',
        description: 'a relay under test',
        contact: 'mailto:ops@example.invalid',
        supported_nips: [1, 9, 11, 16],
      });
      expect(document.toon?.carriage).toBe('btp');
      // TOON_SECRET_KEY wins over its NOSTR_SECRET_KEY alias.
      expect(document['pubkey']).toBe(
        getPublicKey(Uint8Array.from(Buffer.from(DEFAULT_SECRET_KEY, 'hex')))
      );

      const metrics = (await (
        await fetch(`${relay.writeUrl}/metrics`)
      ).json()) as {
        verify: { workers: number };
        ephemeralWriteLane: unknown;
      };
      expect(metrics.verify.workers).toBe(0);
      expect(metrics.ephemeralWriteLane).toEqual({
        enabled: true,
        rateLimit: { maxRequests: 5, windowMs: 2000 },
        maxBodyBytes: 4096,
      });
    },
    { expectedFailureFor: ['rust'] }
  );

  conformanceTest('NOSTR_SECRET_KEY alone sets the identity', async () => {
    const relay = await startRelay(imageUnderTest(), {
      env: { TOON_SECRET_KEY: undefined, NOSTR_SECRET_KEY: '2'.repeat(64) },
    });
    running.push(relay);
    const health = (await (await fetch(`${relay.writeUrl}/health`)).json()) as {
      pubkey: string;
    };
    expect(health.pubkey).toBe(
      getPublicKey(Uint8Array.from(Buffer.from('2'.repeat(64), 'hex')))
    );
  });

  conformanceTest(
    'TOON_MNEMONIC alone sets the identity',
    async () => {
      const relay = await startRelay(imageUnderTest(), {
        env: {
          TOON_SECRET_KEY: undefined,
          TOON_MNEMONIC:
            'abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about',
        },
      });
      running.push(relay);
      const health = (await (
        await fetch(`${relay.writeUrl}/health`)
      ).json()) as {
        pubkey: string;
      };
      expect(health.pubkey).toMatch(/^[0-9a-f]{64}$/);
    },
    { expectedFailureFor: ['rust'] }
  );
});
