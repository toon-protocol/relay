/**
 * Unit tests for the sliding-window rate limiter (relay#129).
 */

import { describe, it, expect } from 'vitest';
import { createRateLimiter } from './rate-limiter.js';

describe('createRateLimiter', () => {
  it('allows requests under the limit and rejects the one that exceeds it', () => {
    const t = 0;
    const limiter = createRateLimiter({
      maxRequests: 3,
      windowMs: 1000,
      now: () => t,
    });

    expect(limiter.allow('k')).toBe(true);
    expect(limiter.allow('k')).toBe(true);
    expect(limiter.allow('k')).toBe(true);
    expect(limiter.allow('k')).toBe(false);
  });

  it('tracks each key independently', () => {
    const t = 0;
    const limiter = createRateLimiter({
      maxRequests: 1,
      windowMs: 1000,
      now: () => t,
    });

    expect(limiter.allow('a')).toBe(true);
    expect(limiter.allow('a')).toBe(false);
    expect(limiter.allow('b')).toBe(true);
  });

  it('admits again once old hits fall outside the trailing window (sliding, not fixed)', () => {
    let t = 0;
    const limiter = createRateLimiter({
      maxRequests: 1,
      windowMs: 1000,
      now: () => t,
    });

    expect(limiter.allow('k')).toBe(true);
    t = 500;
    expect(limiter.allow('k')).toBe(false); // still inside the window
    t = 1001;
    expect(limiter.allow('k')).toBe(true); // the first hit is now outside it
  });

  it('does not let a caller burst 2x across a fixed-window-style boundary', () => {
    // A fixed-window counter would reset hard at t=1000 and let 2 requests
    // land in the ~1ms straddling the boundary (one at 999, one at 1000).
    // The sliding log must not.
    let t = 999;
    const limiter = createRateLimiter({
      maxRequests: 1,
      windowMs: 1000,
      now: () => t,
    });

    expect(limiter.allow('k')).toBe(true);
    t = 1000;
    expect(limiter.allow('k')).toBe(false);
  });

  it('defaults to Date.now when no clock is injected', () => {
    const limiter = createRateLimiter({ maxRequests: 1, windowMs: 1000 });
    expect(limiter.allow('k')).toBe(true);
    expect(limiter.allow('k')).toBe(false);
  });
});
