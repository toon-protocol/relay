/**
 * Worker-thread pool for event-signature verification (relay#85).
 *
 * Post-#87 the WASM verify takes ~0.2ms per event -- but it still runs ON the
 * single Node event loop, which also carries every WebSocket broadcast.
 * Agent writers make persistent-kind write rates potentially bursty and high;
 * a verify burst on the loop is exactly the kind of stall that shows up as
 * tail jitter on ephemeral (huddle-frame) latency. The pool moves persistent
 * -kind schnorr verification onto worker threads so verify bursts cannot
 * stall the loop, at the cost of one thread-hop per verified event.
 *
 * Shape:
 * - `size` workers (default `max(0, os.cpus().length - 1)`); each worker
 *   imports `verify-event.ts` and therefore instantiates its OWN WASM
 *   libsecp256k1 (same self-test + noble fallback semantics as inline).
 * - `size: 0` -- automatic on 1-core boxes, and the explicit config escape
 *   hatch (TOON_VERIFY_WORKERS=0) -- keeps the current inline path: `verify`
 *   resolves synchronously-computed results, no threads are created.
 * - Dispatch is least-busy; per-call results resolve independently.
 *   ORDERING: results for CONCURRENT calls may settle out of submission
 *   order. The write path stays correct because the upstream connector
 *   serializes each BTP session's POSTs (next request only after the
 *   previous response) -- pinned by tests in write-handler.test.ts.
 * - Worker failure degrades transparently: pending and future verifies fall
 *   back to the inline implementation (the pool never hard-fails a write).
 * - Hand-rolled on `node:worker_threads` -- ~100 lines beats a piscina
 *   dependency in a package whose runtime deps are deliberately minimal.
 *
 * @module
 */

import { existsSync } from 'node:fs';
import { cpus } from 'node:os';
import { fileURLToPath } from 'node:url';
import { Worker } from 'node:worker_threads';
import { performance } from 'node:perf_hooks';
import { verifiedSymbol } from 'nostr-tools/pure';
import type { NostrEvent } from 'nostr-tools/pure';
import { verifyEventSignature } from './verify-event.js';

/** A pool that verifies event signatures off the main event loop. */
export interface VerifyPool {
  /**
   * Verify an event's id + BIP-340 signature. Never rejects; invalid or
   * structurally broken events resolve `false`. Semantics match
   * `verifyEventSignature`, including stamping the nostr-tools
   * verified-event cache symbol on the caller's object.
   */
  verify(event: NostrEvent): Promise<boolean>;
  /** Live worker count (0 = inline path). */
  readonly size: number;
  /** Terminate all workers. Idempotent. Pending verifies resolve inline. */
  destroy(): Promise<void>;
}

/**
 * Default pool size: one worker per CPU minus one core reserved for the
 * event loop (WS fan-out + HTTP). On a 1-core box this is 0 -- the inline
 * path -- because a worker would only add thread-hop overhead while
 * competing for the same core.
 */
export function defaultVerifyWorkers(): number {
  return Math.max(0, cpus().length - 1);
}

/** Options for {@link createVerifyPool}. */
export interface VerifyPoolOptions {
  /** Worker count (default {@link defaultVerifyWorkers}; 0 = inline). */
  size?: number;
  /**
   * Called with the wall-clock milliseconds of each verify -- including
   * pool queue + thread-hop time, i.e. the latency a write actually paid.
   * The launcher wires this into the /metrics registry.
   */
  onMeasure?: (ms: number) => void;
}

/**
 * Locate the compiled worker entry. At runtime (dist) it sits next to the
 * bundle; under vitest (src) the repo's build-before-test ordering has
 * already produced `dist/verify-worker.js`. Returns null when no compiled
 * worker exists -- the pool then falls back to the inline path.
 */
function resolveWorkerUrl(): URL | null {
  for (const candidate of [
    new URL('./verify-worker.js', import.meta.url),
    new URL('../../dist/verify-worker.js', import.meta.url),
  ]) {
    try {
      if (existsSync(fileURLToPath(candidate))) return candidate;
    } catch {
      // Non-file URL (unusual bundler context) -- try the next candidate.
    }
  }
  return null;
}

interface PoolWorker {
  worker: Worker;
  pending: Map<number, { resolve: (ok: boolean) => void; event: NostrEvent }>;
}

/**
 * Create a verify pool. See the module doc for semantics; see
 * {@link VerifyPoolOptions} for knobs.
 */
export function createVerifyPool(options: VerifyPoolOptions = {}): VerifyPool {
  const requestedSize = options.size ?? defaultVerifyWorkers();
  const onMeasure = options.onMeasure;

  const measured = <T>(fn: () => T): T => {
    if (!onMeasure) return fn();
    const start = performance.now();
    const result = fn();
    onMeasure(performance.now() - start);
    return result;
  };

  const inlineVerify = (event: NostrEvent): Promise<boolean> =>
    Promise.resolve(measured(() => verifyEventSignature(event)));

  const workerUrl = requestedSize > 0 ? resolveWorkerUrl() : null;
  if (requestedSize > 0 && !workerUrl) {
    console.warn(
      '[relay] verify pool: compiled worker (dist/verify-worker.js) not found -- ' +
        'falling back to inline verification (build the package to enable workers)'
    );
  }

  const workers: PoolWorker[] = [];
  let seq = 0;
  let destroyed = false;

  /** Drop a worker from rotation, resolving its pending verifies inline. */
  const retireWorker = (pw: PoolWorker, reason: string): void => {
    const index = workers.indexOf(pw);
    if (index === -1) return;
    workers.splice(index, 1);
    if (!destroyed) {
      console.warn(
        `[relay] verify pool: worker retired (${reason}); ` +
          (workers.length > 0
            ? `${workers.length} worker(s) remain`
            : 'falling back to inline verification')
      );
    }
    for (const { resolve, event } of pw.pending.values()) {
      resolve(verifyEventSignature(event));
    }
    pw.pending.clear();
  };

  if (workerUrl) {
    for (let i = 0; i < requestedSize; i++) {
      const worker = new Worker(workerUrl);
      const pw: PoolWorker = { worker, pending: new Map() };
      worker.on('message', (reply: { seq: number; ok: boolean }) => {
        const entry = pw.pending.get(reply.seq);
        if (!entry) return;
        pw.pending.delete(reply.seq);
        // Mirror the inline path: stamp the verdict on the caller's object
        // (the worker verified a structured clone).
        entry.event[verifiedSymbol] = reply.ok;
        entry.resolve(reply.ok);
      });
      worker.on('error', (error: Error) =>
        retireWorker(pw, `error: ${error.message}`)
      );
      worker.on('exit', () => retireWorker(pw, 'exit'));
      workers.push(pw);
    }
  }

  const poolVerify = (event: NostrEvent): Promise<boolean> => {
    // Honor a prior verdict without a thread-hop (nostr-tools cache).
    const cached = event[verifiedSymbol];
    if (typeof cached === 'boolean') return Promise.resolve(cached);

    // Least-busy dispatch.
    let target = workers[0];
    if (!target) return inlineVerify(event);
    for (const pw of workers) {
      if (pw.pending.size < target.pending.size) target = pw;
    }

    const start = performance.now();
    return new Promise<boolean>((resolve) => {
      const id = ++seq;
      target.pending.set(id, {
        event,
        resolve: (ok) => {
          onMeasure?.(performance.now() - start);
          resolve(ok);
        },
      });
      target.worker.postMessage({ seq: id, event });
    });
  };

  return {
    verify(event: NostrEvent): Promise<boolean> {
      return workers.length > 0 ? poolVerify(event) : inlineVerify(event);
    },
    get size(): number {
      return workers.length;
    },
    async destroy(): Promise<void> {
      destroyed = true;
      const toTerminate = [...workers];
      for (const pw of toTerminate) {
        retireWorker(pw, 'destroy');
      }
      await Promise.all(toTerminate.map((pw) => pw.worker.terminate()));
    },
  };
}
