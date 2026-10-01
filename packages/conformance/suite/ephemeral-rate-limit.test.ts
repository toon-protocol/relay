import { afterAll, beforeAll, describe, expect } from 'vitest';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { post } from './harness/client.js';
import { conformanceTest, imageUnderTest } from './implementation.js';

// Its own file, so its own relay: exhausting the limiter must not starve the
// ephemeral cases in write.test.ts, which share a client address.
let relay: RunningRelay;

beforeAll(async () => {
  relay = await startRelay(imageUnderTest());
});

afterAll(async () => {
  await relay?.stop();
});

describe('relay image conformance: ephemeral rate limit', () => {
  conformanceTest(
    '429 once a client is over the limit (default 200 per 10s)',
    async () => {
      // The limiter runs before the body is read, so an empty body is enough:
      // it is a 400 until the limit is hit and a 429 after.
      const statuses = new Set<number>();
      for (let sent = 0; sent < 400 && !statuses.has(429); sent += 50) {
        const batch = await Promise.all(
          Array.from({ length: 50 }, () =>
            post(`${relay.writeUrl}/write-ephemeral`, {})
          )
        );
        for (const response of batch) statuses.add(response.status);
      }
      expect(statuses.has(400)).toBe(true);
      expect(statuses.has(429)).toBe(true);
    }
  );
});
