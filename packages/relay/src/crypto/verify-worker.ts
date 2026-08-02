/**
 * Worker-thread entrypoint for the verify pool (relay#85).
 *
 * Each worker imports `verify-event.ts` in its own thread, so each worker
 * instantiates its OWN WASM libsecp256k1 module (with the same load-time
 * self-test and transparent noble fallback as the inline path -- WASM
 * instances are not shareable across threads anyway).
 *
 * Protocol: the parent posts `{ seq, event }`; the worker replies
 * `{ seq, ok }`. `verifyEventSignature` never throws, so every request gets
 * exactly one reply.
 *
 * Built as its own tsup entry (`dist/verify-worker.js`) because
 * `worker_threads` needs a real JS file on disk, not a bundled import.
 *
 * @module
 */

import { parentPort } from 'node:worker_threads';
import type { NostrEvent } from 'nostr-tools/pure';
import { verifyEventSignature } from './verify-event.js';

if (!parentPort) {
  throw new Error('verify-worker must be started as a worker thread');
}

const port = parentPort;
port.on('message', (message: { seq: number; event: NostrEvent }) => {
  port.postMessage({
    seq: message.seq,
    ok: verifyEventSignature(message.event),
  });
});
