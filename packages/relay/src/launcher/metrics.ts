/**
 * Metrics for the relay's HTTP telemetry surface (relay#85).
 *
 * Served as JSON from `GET /metrics` on the write/health port, next to
 * `/health`. Two families, chosen because they are the TRIGGER METRICS for
 * future scaling decisions (2026-08-02 benchmarking, toon-meta
 * proto/spacetimedb-relay RESULTS.md):
 *
 * - **Event-loop delay**: the single Node loop carries every WS broadcast;
 *   loop lag IS ephemeral (huddle-frame) tail latency. Sustained p99 growth
 *   here is the signal to shed load or scale out.
 * - **Per-event verify time**: wall-clock ms per signature verification as
 *   the write actually paid it (including verify-pool queue + thread-hop
 *   when workers are enabled). Growth here is the signal to resize the
 *   verify pool (TOON_VERIFY_WORKERS) or move boxes.
 *
 * The registry is deliberately dependency-free: `monitorEventLoopDelay`
 * from node:perf_hooks plus a fixed-size ring of recent verify durations
 * (percentiles over the last {@link VERIFY_WINDOW} samples -- bounded
 * memory, O(window log window) only when a snapshot is requested).
 *
 * @module
 */

import { monitorEventLoopDelay } from 'node:perf_hooks';
import type { IntervalHistogram } from 'node:perf_hooks';

/** Recent-verify window size (samples) for percentile computation. */
const VERIFY_WINDOW = 2048;

/** Aggregates for one duration family, in milliseconds. */
export interface DurationStats {
  /** Total samples recorded since startup. */
  count: number;
  /** Mean over ALL samples since startup. */
  meanMs: number;
  /** Max over ALL samples since startup. */
  maxMs: number;
  /** Median over the most recent window (up to {@link VERIFY_WINDOW}). */
  p50Ms: number;
  /** 99th percentile over the most recent window. */
  p99Ms: number;
}

/** The `GET /metrics` response shape. */
export interface MetricsSnapshot {
  timestamp: number;
  /**
   * Event-loop delay in ms (node:perf_hooks monitorEventLoopDelay since the
   * last snapshot reset -- lifetime of the process unless noted). `mean`,
   * `p50`, `p99`, `max` -- loop lag is ephemeral-frame tail latency.
   */
  eventLoopDelayMs: {
    mean: number;
    p50: number;
    p99: number;
    max: number;
  };
  /** Per-event signature-verify timing (trigger metric for pool sizing). */
  verify: {
    /** Active implementation: 'libsecp256k1-wasm' or 'noble-pure-js'. */
    implementation: string;
    /** Verify-pool worker count (0 = inline on the event loop). */
    workers: number;
  } & DurationStats;
}

/** Live registry behind `GET /metrics`. */
export interface MetricsRegistry {
  /** Record one verify duration in milliseconds. */
  recordVerify(ms: number): void;
  /** Build the current snapshot (cheap; safe to poll). */
  snapshot(): MetricsSnapshot;
  /** Update the reported worker count (pool may degrade at runtime). */
  setVerifyWorkers(workers: number): void;
  /** Disable the loop-delay histogram (call on relay stop). */
  stop(): void;
}

const NS_PER_MS = 1e6;

function percentileOf(sorted: number[], fraction: number): number {
  if (sorted.length === 0) return 0;
  const index = Math.min(
    sorted.length - 1,
    Math.ceil(fraction * sorted.length) - 1
  );
  return sorted[Math.max(0, index)] ?? 0;
}

function round(value: number): number {
  // The loop-delay histogram reports NaN before its first sample interval;
  // surface 0 rather than a JSON null.
  if (!Number.isFinite(value)) return 0;
  return Math.round(value * 1000) / 1000;
}

/**
 * Create the metrics registry. One per relay instance; the launcher wires
 * `recordVerify` into the verify pool's `onMeasure` and serves `snapshot()`
 * from `GET /metrics`.
 *
 * @param info - Static verify metadata surfaced in the snapshot.
 */
export function createMetricsRegistry(info: {
  verifyImplementation: string;
  verifyWorkers: number;
}): MetricsRegistry {
  const loopDelay: IntervalHistogram = monitorEventLoopDelay({
    resolution: 20,
  });
  loopDelay.enable();

  let verifyWorkers = info.verifyWorkers;
  let count = 0;
  let totalMs = 0;
  let maxMs = 0;
  const window = new Array<number>(VERIFY_WINDOW);
  let windowFill = 0;
  let windowCursor = 0;

  return {
    recordVerify(ms: number): void {
      count += 1;
      totalMs += ms;
      if (ms > maxMs) maxMs = ms;
      window[windowCursor] = ms;
      windowCursor = (windowCursor + 1) % VERIFY_WINDOW;
      if (windowFill < VERIFY_WINDOW) windowFill += 1;
    },

    snapshot(): MetricsSnapshot {
      const recent = window.slice(0, windowFill).sort((a, b) => a - b);
      return {
        timestamp: Date.now(),
        eventLoopDelayMs: {
          mean: round(loopDelay.mean / NS_PER_MS),
          p50: round(loopDelay.percentile(50) / NS_PER_MS),
          p99: round(loopDelay.percentile(99) / NS_PER_MS),
          max: round(loopDelay.max / NS_PER_MS),
        },
        verify: {
          implementation: info.verifyImplementation,
          workers: verifyWorkers,
          count,
          meanMs: round(count > 0 ? totalMs / count : 0),
          maxMs: round(maxMs),
          p50Ms: round(percentileOf(recent, 0.5)),
          p99Ms: round(percentileOf(recent, 0.99)),
        },
      };
    },

    setVerifyWorkers(workers: number): void {
      verifyWorkers = workers;
    },

    stop(): void {
      loopDelay.disable();
    },
  };
}
