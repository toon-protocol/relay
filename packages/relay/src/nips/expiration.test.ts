import { describe, it, expect } from 'vitest';
import { getExpiration, isExpired, EXPIRATION_TAG } from './expiration.js';

describe('NIP-40 expiration parsing', () => {
  it('reads a well-formed expiration tag', () => {
    expect(getExpiration({ tags: [[EXPIRATION_TAG, '1786921252']] })).toBe(
      1786921252
    );
  });

  it('returns undefined when there is no expiration tag', () => {
    expect(
      getExpiration({
        tags: [
          ['d', 'x'],
          ['p', 'abc'],
        ],
      })
    ).toBeUndefined();
  });

  it('ignores other tags while scanning', () => {
    expect(
      getExpiration({
        tags: [
          ['d', ''],
          ['e', 'ff'],
          [EXPIRATION_TAG, '42'],
        ],
      })
    ).toBe(42);
  });

  it('takes the first valid tag when several are present', () => {
    expect(
      getExpiration({
        tags: [
          [EXPIRATION_TAG, '100'],
          [EXPIRATION_TAG, '999'],
        ],
      })
    ).toBe(100);
  });

  // FAIL-OPEN is the whole point: a publisher typo must not become an
  // unrecoverable read outage for that event.
  it.each([
    ['a missing value', [[EXPIRATION_TAG]]],
    ['an empty value', [[EXPIRATION_TAG, '']]],
    ['a non-numeric value', [[EXPIRATION_TAG, 'soon']]],
    ['a float', [[EXPIRATION_TAG, '1786921252.5']]],
    ['a negative value', [[EXPIRATION_TAG, '-1']]],
    ['exponent notation', [[EXPIRATION_TAG, '1e9']]],
    ['hex notation', [[EXPIRATION_TAG, '0x10']]],
    ['surrounding whitespace', [[EXPIRATION_TAG, ' 42 ']]],
  ])('treats %s as never expiring', (_label, tags) => {
    expect(getExpiration({ tags: tags as string[][] })).toBeUndefined();
  });

  it('skips a malformed tag in favour of a later valid one', () => {
    expect(
      getExpiration({
        tags: [
          [EXPIRATION_TAG, 'soon'],
          [EXPIRATION_TAG, '7'],
        ],
      })
    ).toBe(7);
  });
});

describe('NIP-40 isExpired', () => {
  it('is false before the expiration', () => {
    expect(isExpired({ tags: [[EXPIRATION_TAG, '100']] }, 99)).toBe(false);
  });

  it('is true at the expiration second', () => {
    expect(isExpired({ tags: [[EXPIRATION_TAG, '100']] }, 100)).toBe(true);
  });

  it('is true after the expiration', () => {
    expect(isExpired({ tags: [[EXPIRATION_TAG, '100']] }, 101)).toBe(true);
  });

  it('is never true for an event without the tag', () => {
    expect(isExpired({ tags: [] }, Number.MAX_SAFE_INTEGER)).toBe(false);
  });
});
