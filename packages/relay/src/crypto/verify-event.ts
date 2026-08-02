/**
 * Event-signature verification for the paid-write hot path (relay#85).
 *
 * nostr-tools' `verifyEvent` runs BIP-340 schnorr verification in pure-JS
 * noble at ~1.3ms per event -- synchronously, on the relay's single Node
 * event loop. Post-#84 that verify IS the write-path ceiling: the devnet
 * relay caps at ~240-260 events/s aggregate, CPU-bound (connector#685
 * Phase G measurements).
 *
 * This module verifies with libsecp256k1 compiled to WASM (`tiny-secp256k1`,
 * ~0.20ms per event on the same workload -- ~7x faster) and keeps noble as a
 * fallback: if the WASM module fails to load or fails a startup self-test on
 * a known-good/known-bad vector pair, verification transparently degrades to
 * `verifyEvent` from nostr-tools. The relay never hard-fails because of the
 * fast path. WASM was chosen over a native addon deliberately: it needs no
 * platform toolchain in the Docker build, and the `secp256k1` native-addon
 * package exposes no schnorr API at all.
 *
 * Semantics match nostr-tools `verifyEvent` -- serialize per NIP-01, SHA-256,
 * compare against `event.id`, then BIP-340-verify `event.sig` -- with one
 * deliberate improvement: a structurally invalid event returns `false` here
 * (nostr-tools' `getEventHash` throws instead). `verifyEventSignature` never
 * throws.
 *
 * @module
 */

import { createHash } from 'node:crypto';
import { createRequire } from 'node:module';
import {
  serializeEvent,
  verifiedSymbol,
  verifyEvent as nobleVerifyEvent,
} from 'nostr-tools/pure';
import type { NostrEvent } from 'nostr-tools/pure';

/**
 * BIP-340 verify: 32-byte message hash, 32-byte x-only pubkey, 64-byte
 * signature. Shape of `tiny-secp256k1`'s `verifySchnorr`.
 */
type VerifySchnorr = (
  hash: Uint8Array,
  pubkey: Uint8Array,
  signature: Uint8Array
) => boolean;

/**
 * Deterministic self-test vector: a valid signed event and a tampered copy.
 * Used at load time to prove the WASM verifier actually works on this
 * platform before it is trusted on the hot path.
 */
const SELF_TEST_EVENT: NostrEvent = {
  kind: 1,
  created_at: 1754000000,
  tags: [],
  content: 'toon-relay fast-verify self-test',
  pubkey: '6a04ab98d9e4774ad806e302dddeb63bea16b5cb5f223ee77478e861bb583eb3',
  id: 'ff38bb1eb0dbd275934b16675751e8ed547812b7ca8c0cf7fe89ca666c85176a',
  sig: 'faa9977f43a6b5bd183b41963658f5258c22c8b7a60ad99779d4d82ddaadb53682a851f55f855f40c7faae7fd00c4a71eed14f3fdfb60a2751fe1b2a414dbbd3',
};

/**
 * Verify an event with a given schnorr implementation: NIP-01 serialize,
 * SHA-256 (Node's native hash, not pure JS), id check, then BIP-340 verify.
 * Returns false (never throws) on structurally invalid input.
 */
function verifyWith(verifySchnorr: VerifySchnorr, event: NostrEvent): boolean {
  try {
    // Mirror nostr-tools' verified-event cache: honor a prior verdict and
    // stamp ours, so downstream code that re-verifies (or deep-compares) the
    // event sees exactly what noble's verifyEvent would have produced.
    if (typeof event[verifiedSymbol] === 'boolean') {
      return event[verifiedSymbol];
    }
    if (typeof event.id !== 'string' || typeof event.sig !== 'string') {
      return false;
    }
    // serializeEvent validates structure (types, pubkey format) and throws on
    // bad input -- caught below. Reusing it guarantees byte-parity with the
    // noble path's getEventHash forever.
    const hash = createHash('sha256')
      .update(serializeEvent(event), 'utf8')
      .digest();
    if (hash.toString('hex') !== event.id) {
      return (event[verifiedSymbol] = false);
    }
    // event.sig must be exactly 64 hex-decoded bytes. Buffer.from(_, 'hex')
    // stops at the first invalid character, so the length check also rejects
    // non-hex input; the explicit string-length check rejects odd/overlong
    // strings that would still decode to 64 bytes.
    if (event.sig.length !== 128) {
      return false;
    }
    const sig = Buffer.from(event.sig, 'hex');
    const pubkey = Buffer.from(event.pubkey, 'hex');
    if (sig.length !== 64 || pubkey.length !== 32) {
      return false;
    }
    return (event[verifiedSymbol] = verifySchnorr(hash, pubkey, sig));
  } catch {
    return false;
  }
}

/**
 * Load tiny-secp256k1 (synchronous WASM instantiation) and prove it against
 * the self-test vector pair. Returns null -- meaning "use the noble
 * fallback" -- on any load or self-test failure.
 */
function loadFastVerifier(): VerifySchnorr | null {
  try {
    const require = createRequire(import.meta.url);
    const tiny = require('tiny-secp256k1') as {
      verifySchnorr?: VerifySchnorr;
    };
    const candidate = tiny.verifySchnorr;
    if (typeof candidate !== 'function') {
      return null;
    }
    const tampered: NostrEvent = {
      ...SELF_TEST_EVENT,
      sig: '0'.repeat(128),
    };
    if (
      !verifyWith(candidate, SELF_TEST_EVENT) ||
      verifyWith(candidate, tampered)
    ) {
      return null;
    }
    return candidate;
  } catch {
    return null;
  }
}

const fastVerifySchnorr = loadFastVerifier();

/**
 * Which verify implementation is active. Surfaced so the launcher can log it
 * once at startup (the noble fallback is a silent ~7x throughput loss
 * otherwise).
 */
export const verifyImplementation: 'libsecp256k1-wasm' | 'noble-pure-js' =
  fastVerifySchnorr ? 'libsecp256k1-wasm' : 'noble-pure-js';

/**
 * Verify ONLY that `event.id` is the correct NIP-01 SHA-256 hash of the
 * event's serialized form. Does NOT check the schnorr signature and does NOT
 * stamp nostr-tools' `verifiedSymbol` cache (an id check is not a signature
 * verdict).
 *
 * This is the integrity floor for the paid-ephemeral skip-verify path
 * (relay#85): when the relay skips schnorr for payment-gated ephemeral kinds,
 * it still refuses events whose id does not match their content, so a paid
 * writer cannot make the relay broadcast a frame whose bytes disagree with
 * the id that clients index/verify by.
 *
 * Never throws; structurally invalid events return false.
 *
 * @param event - The event whose id to check.
 * @returns True iff `event.id` equals the NIP-01 SHA-256 of the event.
 */
export function verifyEventId(event: NostrEvent): boolean {
  try {
    if (typeof event.id !== 'string') {
      return false;
    }
    // serializeEvent validates structure and throws on bad input -- caught
    // below. Same serializer as the full-verify path, so the two checks can
    // never disagree about what bytes an id covers.
    const hash = createHash('sha256')
      .update(serializeEvent(event), 'utf8')
      .digest();
    return hash.toString('hex') === event.id;
  } catch {
    return false;
  }
}

/**
 * Verify a Nostr event's id and BIP-340 signature.
 *
 * Drop-in replacement for nostr-tools' `verifyEvent` on the write hot path:
 * same accept/reject semantics for well-formed events, ~7x faster via WASM
 * libsecp256k1 when available, noble pure-JS otherwise. Never throws;
 * structurally invalid events return false.
 *
 * @param event - The event to verify.
 * @returns True iff the id matches the NIP-01 hash and the signature is valid.
 */
export function verifyEventSignature(event: NostrEvent): boolean {
  if (fastVerifySchnorr) {
    return verifyWith(fastVerifySchnorr, event);
  }
  try {
    return nobleVerifyEvent(event);
  } catch {
    return false;
  }
}
