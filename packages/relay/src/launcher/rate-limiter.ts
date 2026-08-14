/**
 * A minimal in-memory sliding-window rate limiter (relay#129).
 *
 * Built for the free ephemeral write lane (`POST /write-ephemeral`,
 * handlers/write-ephemeral-handler.ts): with no payment gate, request volume
 * is the only admission control that lane has, so every request must be
 * checked against a per-key budget before any other work happens.
 * Dependency-free and small enough to own outright rather than pull in a
 * library, matching the shape the rest of the launcher already uses for
 * self-contained stateful helpers (crypto/verify-pool.ts, launcher/metrics.ts).
 *
 * Sliding-window LOG, not a fixed-window counter: a fixed window lets a
 * caller burst up to 2x the limit across a window boundary (all of window
 * N's budget at :59, all of window N+1's budget at :00). The log costs one
 * array per active key and O(requests-in-window) work per call, which is
 * fine at the request volumes a bound like this is meant to police.
 *
 * @module
 */

/** Options for {@link createRateLimiter}. */
export interface RateLimiterOptions {
  /** Max requests allowed per key within the trailing window. */
  maxRequests: number;
  /** Trailing window length in milliseconds. */
  windowMs: number;
  /** Clock override for deterministic tests (default: `Date.now`). */
  now?: () => number;
}

/** A live rate limiter. One instance per bound; keys are caller-defined. */
export interface RateLimiter {
  /**
   * Check `key`'s budget. Returns `true` and records this call towards the
   * budget when the key is under its limit; returns `false` (and records
   * nothing) when the key is over budget. Never throws.
   */
  allow(key: string): boolean;
}

/**
 * Create a sliding-window rate limiter.
 *
 * @param options - Bound + optional clock override.
 */
export function createRateLimiter(options: RateLimiterOptions): RateLimiter {
  const { maxRequests, windowMs } = options;
  const now = options.now ?? Date.now;
  const hits = new Map<string, number[]>();

  return {
    allow(key: string): boolean {
      const t = now();
      const windowStart = t - windowMs;
      // Drop hits that aged out of the trailing window, then keep the pruned
      // log either way -- a rejected call still shrinks the key's entry.
      const timestamps = (hits.get(key) ?? []).filter(
        (ts) => ts >= windowStart
      );
      hits.set(key, timestamps);

      if (timestamps.length >= maxRequests) {
        return false;
      }

      timestamps.push(t);
      return true;
    },
  };
}
