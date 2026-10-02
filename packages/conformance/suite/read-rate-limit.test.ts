import { afterAll, beforeAll, describe, expect } from 'vitest';
import { Client } from './harness/client.js';
import {
  runRelayToExit,
  startRelay,
  type RunningRelay,
} from './harness/relay-container.js';
import { conformanceTest, imageUnderTest } from './implementation.js';

// Its own file, and two relays of its own: exhausting a read allowance must
// not starve the other cases, which share this client's address.
const PER_CONNECTION = 5;
const PER_SOURCE = 8;
/** An id no test ever stores, for REQs that only need an EOSE. */
const NO_SUCH_ID = '0'.repeat(64);

let perConnection: RunningRelay;
let perSource: RunningRelay;

beforeAll(async () => {
  perConnection = await startRelay(imageUnderTest(), {
    env: {
      TOON_READ_RATE_LIMIT: String(PER_CONNECTION),
      TOON_READ_SOURCE_RATE_LIMIT: '100000',
    },
  });
  perSource = await startRelay(imageUnderTest(), {
    env: {
      TOON_READ_RATE_LIMIT: '100000',
      TOON_READ_SOURCE_RATE_LIMIT: String(PER_SOURCE),
    },
  });
});

afterAll(async () => {
  await perConnection?.stop();
  await perSource?.stop();
});

/** Send `count` REQs, one after another, and return what each was answered. */
async function ask(client: Client, count: number): Promise<string[]> {
  const answers: string[] = [];
  for (let n = 0; n < count; n++) {
    const id = `s${n}`;
    client.send(['REQ', id, { ids: [NO_SUCH_ID] }]);
    const frame = await client.next((f) => f[1] === id);
    answers.push(frame[0] === 'CLOSED' ? String(frame[2]) : frame[0]);
  }
  return answers;
}

const SLOW_DOWN = /^rate-limited: .*slow down.*subscribe/i;

describe('relay image conformance: free read rate limit', () => {
  conformanceTest(
    'a connection past its REQ allowance is CLOSED with words that say slow down or subscribe, and stays open',
    async () => {
      const client = await Client.connect(perConnection.readWsUrl);
      try {
        const answers = await ask(client, PER_CONNECTION + 3);
        expect(answers.slice(0, PER_CONNECTION)).toEqual(
          Array(PER_CONNECTION).fill('EOSE')
        );
        for (const refused of answers.slice(PER_CONNECTION)) {
          expect(refused).toMatch(SLOW_DOWN);
        }
        expect(client.closed).toBeUndefined();
      } finally {
        client.close();
      }
    },
    { expectedFailureFor: ['typescript'] }
  );

  conformanceTest(
    'the allowance is per connection: another connection from the same source is answered',
    async () => {
      const first = await Client.connect(perConnection.readWsUrl);
      const second = await Client.connect(perConnection.readWsUrl);
      try {
        await ask(first, PER_CONNECTION + 1);
        expect(await ask(second, 1)).toEqual(['EOSE']);
      } finally {
        first.close();
        second.close();
      }
    },
    { expectedFailureFor: ['typescript'] }
  );

  conformanceTest(
    'the allowance is per source too: connections from one address share one, and a fresh connection does not reset it',
    async () => {
      const answers: string[] = [];
      for (let n = 0; n < PER_SOURCE + 3; n++) {
        const client = await Client.connect(perSource.readWsUrl);
        try {
          answers.push(...(await ask(client, 1)));
        } finally {
          client.close();
        }
      }
      expect(answers.slice(0, PER_SOURCE)).toEqual(
        Array(PER_SOURCE).fill('EOSE')
      );
      for (const refused of answers.slice(PER_SOURCE)) {
        expect(refused).toMatch(SLOW_DOWN);
      }
    },
    { expectedFailureFor: ['typescript'] }
  );

  conformanceTest(
    'the limits are advertised in the information document, per minute',
    async () => {
      const response = await fetch(perConnection.readUrl, {
        headers: { accept: 'application/nostr+json' },
      });
      const { limitation } = (await response.json()) as {
        limitation: Record<string, unknown>;
      };
      expect(limitation['max_req_per_minute_per_connection']).toBe(
        PER_CONNECTION
      );
      expect(limitation['max_req_per_minute_per_source']).toBe(100000);
    },
    { expectedFailureFor: ['typescript'] }
  );

  conformanceTest(
    'a limit that is not a positive integer refuses to start',
    async () => {
      for (const name of [
        'TOON_READ_RATE_LIMIT',
        'TOON_READ_SOURCE_RATE_LIMIT',
      ]) {
        const exited = await runRelayToExit(imageUnderTest(), {
          env: { TOON_SECRET_KEY: '1'.repeat(64), [name]: '0' },
        });
        expect(exited.code).not.toBe(0);
        expect(exited.output).toContain(name);
      }
    },
    { expectedFailureFor: ['typescript'] }
  );
});
