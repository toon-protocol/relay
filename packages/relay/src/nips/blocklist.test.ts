import { describe, it, expect } from 'vitest';
import { parseBlockedEventIds } from './blocklist.js';

const ID_A = 'a'.repeat(64);
const ID_B = 'b'.repeat(64);

describe('operator blocklist parsing', () => {
  it('is empty when unset', () => {
    expect(parseBlockedEventIds(undefined)).toEqual({ ids: [], invalid: [] });
    expect(parseBlockedEventIds('')).toEqual({ ids: [], invalid: [] });
  });

  it('accepts a comma-separated list', () => {
    expect(parseBlockedEventIds(`${ID_A},${ID_B}`).ids).toEqual([ID_A, ID_B]);
  });

  it('accepts whitespace and trailing separators', () => {
    expect(parseBlockedEventIds(` ${ID_A} , ${ID_B}, `).ids).toEqual([
      ID_A,
      ID_B,
    ]);
  });

  it('normalizes case', () => {
    expect(parseBlockedEventIds('A'.repeat(64)).ids).toEqual([ID_A]);
  });

  it('de-duplicates', () => {
    expect(parseBlockedEventIds(`${ID_A},${ID_A}`).ids).toEqual([ID_A]);
  });

  // A typo must be LOUD. The silent failure mode -- an operator who believes
  // an event is blocked while the relay keeps serving it -- is the bad one.
  it('reports malformed entries instead of dropping them', () => {
    const result = parseBlockedEventIds(`${ID_A},not-an-id,${'f'.repeat(63)}`);
    expect(result.ids).toEqual([ID_A]);
    expect(result.invalid).toEqual(['not-an-id', 'f'.repeat(63)]);
  });

  // Pubkeys are the same 64-hex shape as ids, so parsing cannot tell them
  // apart; the id-only scope is enforced by what the store does with the
  // list (it matches `events.id`, never `events.pubkey`) -- see
  // retention.test.ts.
  it('has no separate pubkey form', () => {
    expect(parseBlockedEventIds(`npub1${'x'.repeat(58)}`).ids).toEqual([]);
  });
});
