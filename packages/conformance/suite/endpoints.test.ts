import { afterAll, beforeAll, describe, expect } from 'vitest';
import { startRelay, type RunningRelay } from './harness/relay-container.js';
import { conformanceTest, imageUnderTest } from './implementation.js';

let relay: RunningRelay;

beforeAll(async () => {
  relay = await startRelay(imageUnderTest());
});

afterAll(async () => {
  await relay?.stop();
});

describe('relay image conformance: operational endpoints', () => {
  conformanceTest(
    'GET /health returns status, pubkey, capabilities, version and timestamp',
    async () => {
      const response = await fetch(`${relay.writeUrl}/health`);
      expect(response.status).toBe(200);
      expect(await response.json()).toEqual({
        status: 'healthy',
        pubkey: expect.stringMatching(/^[0-9a-f]{64}$/),
        capabilities: expect.arrayContaining(['relay']),
        version: expect.any(String),
        timestamp: expect.any(Number),
      });
    }
  );

  conformanceTest(
    'GET /metrics returns loop delay, verify timings and the free lane bounds',
    async () => {
      const response = await fetch(`${relay.writeUrl}/metrics`);
      expect(response.status).toBe(200);
      expect(await response.json()).toEqual({
        timestamp: expect.any(Number),
        eventLoopDelayMs: {
          mean: expect.any(Number),
          p50: expect.any(Number),
          p99: expect.any(Number),
          max: expect.any(Number),
        },
        verify: {
          implementation: expect.any(String),
          workers: expect.any(Number),
          count: expect.any(Number),
          meanMs: expect.any(Number),
          maxMs: expect.any(Number),
          p50Ms: expect.any(Number),
          p99Ms: expect.any(Number),
        },
        ephemeralWriteLane: {
          enabled: true,
          rateLimit: { maxRequests: 200, windowMs: 10_000 },
          maxBodyBytes: 8192,
        },
      });
    }
  );
});
