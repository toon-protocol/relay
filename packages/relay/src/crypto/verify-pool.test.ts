/**
 * Tests for the worker-thread verify pool (relay#85).
 *
 * NOTE: pooled tests need the compiled worker at
 * `packages/relay/dist/verify-worker.js` -- the repo gate builds before
 * testing (same ordering the typecheck already requires). If the pooled
 * tests fail with size 0, run `pnpm -r build` first.
 */

import { describe, it, expect, afterEach } from 'vitest';
import { cpus } from 'node:os';
import {
  finalizeEvent,
  generateSecretKey,
  verifiedSymbol,
} from 'nostr-tools/pure';
import type { NostrEvent } from 'nostr-tools/pure';
import { createVerifyPool, defaultVerifyWorkers } from './verify-pool.js';
import type { VerifyPool } from './verify-pool.js';

function signedEvent(overrides: Partial<NostrEvent> = {}): NostrEvent {
  const sk = generateSecretKey();
  const event = finalizeEvent(
    {
      kind: 1,
      content: 'verify pool test',
      tags: [],
      created_at: 1754000000,
      ...overrides,
    },
    sk
  );
  // Strip finalizeEvent's verified-cache stamp so the pool does real work.
  return JSON.parse(JSON.stringify(event)) as NostrEvent;
}

let pool: VerifyPool | undefined;

afterEach(async () => {
  if (pool) {
    await pool.destroy();
    pool = undefined;
  }
});

describe('defaultVerifyWorkers', () => {
  it('is cpus - 1, floored at 0 (inline on 1-core boxes)', () => {
    expect(defaultVerifyWorkers()).toBe(Math.max(0, cpus().length - 1));
  });
});

describe('createVerifyPool (inline, size 0)', () => {
  it('verifies without creating workers', async () => {
    pool = createVerifyPool({ size: 0 });
    expect(pool.size).toBe(0);
    await expect(pool.verify(signedEvent())).resolves.toBe(true);
    await expect(
      pool.verify({ ...signedEvent(), sig: '0'.repeat(128) })
    ).resolves.toBe(false);
  });

  it('reports verify durations via onMeasure', async () => {
    const durations: number[] = [];
    pool = createVerifyPool({ size: 0, onMeasure: (ms) => durations.push(ms) });
    await pool.verify(signedEvent());
    expect(durations).toHaveLength(1);
    expect(durations[0]).toBeGreaterThanOrEqual(0);
  });
});

describe('createVerifyPool (worker threads)', () => {
  it('spins up workers and verifies valid/invalid events off the loop', async () => {
    pool = createVerifyPool({ size: 2 });
    // If this is 0, the compiled worker was missing -- build before testing.
    expect(pool.size).toBe(2);

    await expect(pool.verify(signedEvent())).resolves.toBe(true);
    await expect(
      pool.verify({ ...signedEvent(), sig: '0'.repeat(128) })
    ).resolves.toBe(false);
    await expect(
      pool.verify({ ...signedEvent(), content: 'tampered' })
    ).resolves.toBe(false);
    // Never rejects, even on structurally invalid input.
    await expect(pool.verify({} as NostrEvent)).resolves.toBe(false);
  });

  it('stamps the nostr-tools verified-event cache on the caller object', async () => {
    pool = createVerifyPool({ size: 1 });
    expect(pool.size).toBe(1);
    const event = signedEvent();
    await expect(pool.verify(event)).resolves.toBe(true);
    expect(event[verifiedSymbol]).toBe(true);
    // The cache short-circuits the thread-hop on re-verify.
    await expect(pool.verify(event)).resolves.toBe(true);
  });

  it('resolves each concurrent verify with ITS event verdict (mixed batch)', async () => {
    pool = createVerifyPool({ size: 2 });
    expect(pool.size).toBe(2);
    const jobs = Array.from({ length: 24 }, (_, i) => {
      const valid = i % 3 !== 0;
      const event = valid
        ? signedEvent({ created_at: 1754000000 + i })
        : {
            ...signedEvent({ created_at: 1754000000 + i }),
            sig: '0'.repeat(128),
          };
      return {
        valid,
        promise: undefined as Promise<boolean> | undefined,
        event,
      };
    });
    for (const job of jobs) {
      job.promise = pool.verify(job.event);
    }
    const results = await Promise.all(jobs.map((j) => j.promise));
    results.forEach((ok, i) => {
      expect(ok, `job ${i}`).toBe(jobs[i]?.valid);
    });
  });

  it('sequential awaited verifies resolve in submission order (the connector-serialization contract)', async () => {
    // Per-session write ordering is enforced UPSTREAM: the connector does
    // not POST a session's next write until the previous response arrived.
    // This test documents and guards the relay-side half of that contract:
    // awaiting each verify before submitting the next can never reorder.
    pool = createVerifyPool({ size: 2 });
    expect(pool.size).toBe(2);
    const order: number[] = [];
    for (let i = 0; i < 10; i++) {
      const ok = await pool.verify(signedEvent({ created_at: 1754000000 + i }));
      expect(ok).toBe(true);
      order.push(i);
    }
    expect(order).toEqual([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
  });

  it('measures pooled verifies (queue + thread-hop included)', async () => {
    const durations: number[] = [];
    pool = createVerifyPool({ size: 1, onMeasure: (ms) => durations.push(ms) });
    expect(pool.size).toBe(1);
    await pool.verify(signedEvent());
    expect(durations).toHaveLength(1);
    expect(durations[0]).toBeGreaterThan(0);
  });

  it('destroy() terminates workers and later verifies fall back inline', async () => {
    const p = createVerifyPool({ size: 1 });
    expect(p.size).toBe(1);
    await p.destroy();
    expect(p.size).toBe(0);
    // Still correct, now inline.
    await expect(p.verify(signedEvent())).resolves.toBe(true);
    await p.destroy(); // idempotent
  });
});
