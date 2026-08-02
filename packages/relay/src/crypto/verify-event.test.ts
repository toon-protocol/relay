/**
 * Unit tests for the fast event-signature verifier (relay#85).
 *
 * The module replaces nostr-tools' pure-JS noble verify on the write hot
 * path with WASM libsecp256k1 (tiny-secp256k1), keeping noble as a load-time
 * fallback. These tests pin:
 *
 * - the fast path actually loaded on this platform (it must in CI/Docker --
 *   the fallback exists for exotic platforms, not for the supported ones)
 * - accept/reject parity with nostr-tools' verifyEvent across valid events,
 *   tampered content, tampered ids, forged signatures, and wrong-key
 *   signatures
 * - never-throws semantics on structurally invalid input (noble's
 *   getEventHash throws; this module returns false)
 */

import { describe, it, expect } from 'vitest';
import {
  finalizeEvent,
  generateSecretKey,
  getEventHash,
  verifiedSymbol,
  verifyEvent as nobleVerifyEvent,
} from 'nostr-tools/pure';
import type { NostrEvent } from 'nostr-tools/pure';
import { verifyEventSignature, verifyImplementation } from './verify-event.js';

function signedEvent(overrides: Partial<NostrEvent> = {}): NostrEvent {
  const sk = generateSecretKey();
  return finalizeEvent(
    {
      kind: 20001,
      content: 'x'.repeat(140),
      tags: [['s', 'huddle-0']],
      created_at: 1754000000,
      ...overrides,
    },
    sk
  );
}

/** Deep-copy an event to strip nostr-tools' verifiedSymbol cache. */
function fresh(event: NostrEvent): NostrEvent {
  return JSON.parse(JSON.stringify(event)) as NostrEvent;
}

describe('verifyEventSignature', () => {
  it('uses the WASM libsecp256k1 fast path on supported platforms', () => {
    // The noble fallback is for platforms where the WASM module cannot load.
    // CI and the alpine Docker image are supported platforms: if this fails,
    // the hot path silently lost its ~7x speedup.
    expect(verifyImplementation).toBe('libsecp256k1-wasm');
  });

  it('accepts a valid signed event (regular and ephemeral kinds)', () => {
    expect(verifyEventSignature(signedEvent({ kind: 1 }))).toBe(true);
    expect(verifyEventSignature(signedEvent({ kind: 20001 }))).toBe(true);
  });

  it('rejects tampered content (id no longer matches)', () => {
    // fresh() strips finalizeEvent's verified-cache stamp, which object
    // spread would otherwise copy onto the tampered event (noble's
    // verifyEvent honors that stamp identically).
    const event = fresh(signedEvent());
    const tampered = { ...event, content: event.content + '!' };
    expect(verifyEventSignature(tampered)).toBe(false);
  });

  it('rejects a re-hashed tampered event (id matches, signature does not)', () => {
    const event = fresh(signedEvent());
    const tampered = { ...event, content: event.content + '!' };
    tampered.id = getEventHash(tampered);
    expect(verifyEventSignature(tampered)).toBe(false);
  });

  it('rejects a forged signature and a signature from the wrong key', () => {
    const event = fresh(signedEvent());
    expect(verifyEventSignature({ ...event, sig: '0'.repeat(128) })).toBe(
      false
    );

    const other = signedEvent({ created_at: event.created_at });
    expect(verifyEventSignature({ ...event, sig: other.sig })).toBe(false);
  });

  it('rejects malformed sig encodings without throwing', () => {
    const event = fresh(signedEvent());
    expect(verifyEventSignature({ ...event, sig: 'zz'.repeat(64) })).toBe(
      false
    );
    expect(verifyEventSignature({ ...event, sig: event.sig.slice(1) })).toBe(
      false
    );
    expect(verifyEventSignature({ ...event, sig: event.sig + 'ab' })).toBe(
      false
    );
  });

  it('returns false (never throws) on structurally invalid events', () => {
    const cases = [
      {},
      { kind: 1 },
      { ...fresh(signedEvent()), tags: 'not-an-array' },
      { ...fresh(signedEvent()), pubkey: 'nothex' },
      { ...fresh(signedEvent()), created_at: 'yesterday' },
    ] as unknown as NostrEvent[];
    for (const bad of cases) {
      expect(() => verifyEventSignature(bad)).not.toThrow();
      expect(verifyEventSignature(bad)).toBe(false);
    }
  });

  it('stamps and honors the nostr-tools verified-event cache symbol', () => {
    // Stamps its verdict like noble's verifyEvent does...
    const event = fresh(signedEvent());
    expect(verifyEventSignature(event)).toBe(true);
    expect(event[verifiedSymbol]).toBe(true);

    // ...and honors a prior verdict without re-verifying.
    const cached = fresh(signedEvent());
    cached[verifiedSymbol] = false;
    expect(verifyEventSignature(cached)).toBe(false);
  });

  it('agrees with nostr-tools verifyEvent across valid and mutated events', () => {
    for (let i = 0; i < 20; i++) {
      const event = signedEvent({ created_at: 1754000000 + i });
      const mutations: NostrEvent[] = [
        event,
        { ...event, content: event.content + i },
        { ...event, sig: '0'.repeat(128) },
        { ...event, id: '0'.repeat(64) },
      ];
      for (const candidate of mutations) {
        // fresh() strips the verifiedSymbol cache so noble does real work.
        expect(verifyEventSignature(fresh(candidate))).toBe(
          nobleVerifyEvent(fresh(candidate))
        );
      }
    }
  });
});
