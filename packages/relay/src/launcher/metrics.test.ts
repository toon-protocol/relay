/**
 * Unit tests for the metrics registry behind GET /metrics (relay#85).
 *
 * Event-loop lag and per-event verify time are the trigger metrics for
 * scaling decisions; these tests pin the snapshot shape and the verify
 * aggregation math (count/mean/max over lifetime, p50/p99 over the recent
 * window).
 */

import { describe, it, expect } from 'vitest';
import { createMetricsRegistry } from './metrics.js';

describe('createMetricsRegistry', () => {
  it('produces a well-formed snapshot with zero recorded verifies', () => {
    const registry = createMetricsRegistry({
      verifyImplementation: 'libsecp256k1-wasm',
      verifyWorkers: 3,
      ephemeralRateLimit: { maxRequests: 200, windowMs: 10_000 },
      ephemeralMaxBodyBytes: 8192,
    });
    try {
      const snap = registry.snapshot();
      expect(snap.timestamp).toBeGreaterThan(0);
      expect(snap.verify.implementation).toBe('libsecp256k1-wasm');
      expect(snap.verify.workers).toBe(3);
      expect(snap.verify.count).toBe(0);
      expect(snap.verify.meanMs).toBe(0);
      expect(snap.verify.p50Ms).toBe(0);
      expect(snap.verify.p99Ms).toBe(0);
      expect(snap.verify.maxMs).toBe(0);
      // Loop-delay numbers must be finite JSON numbers (the histogram
      // reports NaN before its first interval; the registry surfaces 0).
      for (const value of Object.values(snap.eventLoopDelayMs)) {
        expect(Number.isFinite(value)).toBe(true);
      }
      // Ephemeral write lane bounds (relay#129) -- always present, always
      // enabled, and static for the process lifetime.
      expect(snap.ephemeralWriteLane).toEqual({
        enabled: true,
        rateLimit: { maxRequests: 200, windowMs: 10_000 },
        maxBodyBytes: 8192,
      });
    } finally {
      registry.stop();
    }
  });

  it('aggregates verify durations: count/mean/max lifetime, percentiles over the window', () => {
    const registry = createMetricsRegistry({
      verifyImplementation: 'noble-pure-js',
      verifyWorkers: 0,
      ephemeralRateLimit: { maxRequests: 200, windowMs: 10_000 },
      ephemeralMaxBodyBytes: 8192,
    });
    try {
      for (const ms of [1, 2, 3, 4, 100]) {
        registry.recordVerify(ms);
      }
      const snap = registry.snapshot();
      expect(snap.verify.count).toBe(5);
      expect(snap.verify.meanMs).toBe(22);
      expect(snap.verify.maxMs).toBe(100);
      expect(snap.verify.p50Ms).toBe(3);
      expect(snap.verify.p99Ms).toBe(100);
    } finally {
      registry.stop();
    }
  });

  it('setVerifyWorkers updates the reported pool size (runtime degradation)', () => {
    const registry = createMetricsRegistry({
      verifyImplementation: 'libsecp256k1-wasm',
      verifyWorkers: 4,
      ephemeralRateLimit: { maxRequests: 200, windowMs: 10_000 },
      ephemeralMaxBodyBytes: 8192,
    });
    try {
      registry.setVerifyWorkers(1);
      expect(registry.snapshot().verify.workers).toBe(1);
    } finally {
      registry.stop();
    }
  });

  it('keeps percentiles bounded to the recent window under sustained load', () => {
    const registry = createMetricsRegistry({
      verifyImplementation: 'libsecp256k1-wasm',
      verifyWorkers: 2,
      ephemeralRateLimit: { maxRequests: 200, windowMs: 10_000 },
      ephemeralMaxBodyBytes: 8192,
    });
    try {
      // One early outlier, then a long steady stream that evicts it from
      // the window: max stays lifetime, p99 tracks the recent stream.
      registry.recordVerify(500);
      for (let i = 0; i < 4096; i++) {
        registry.recordVerify(1);
      }
      const snap = registry.snapshot();
      expect(snap.verify.maxMs).toBe(500);
      expect(snap.verify.p99Ms).toBe(1);
      expect(snap.verify.count).toBe(4097);
    } finally {
      registry.stop();
    }
  });
});
