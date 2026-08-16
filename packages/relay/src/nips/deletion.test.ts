import { describe, it, expect } from 'vitest';
import {
  DELETION_KIND,
  isDeletionKind,
  isDeletableBy,
  parseAddressCoordinate,
  parseDeletionTargets,
} from './deletion.js';

const AUTHOR = 'a'.repeat(64);
const STRANGER = 'b'.repeat(64);
const EVENT_ID = 'c'.repeat(64);

describe('isDeletionKind', () => {
  it('recognizes kind 5 only', () => {
    expect(isDeletionKind(DELETION_KIND)).toBe(true);
    expect(isDeletionKind(1)).toBe(false);
    expect(isDeletionKind(10032)).toBe(false);
  });
});

describe('parseAddressCoordinate', () => {
  it('parses a coordinate', () => {
    expect(parseAddressCoordinate(`10032:${AUTHOR}:`)).toEqual({
      kind: 10032,
      pubkey: AUTHOR,
      identifier: '',
    });
  });

  it('keeps colons inside the identifier', () => {
    expect(parseAddressCoordinate(`30023:${AUTHOR}:a:b:c`)?.identifier).toBe(
      'a:b:c'
    );
  });

  it.each([
    ['no separators', 'nonsense'],
    ['one separator', `10032:${AUTHOR}`],
    ['a non-numeric kind', `kind:${AUTHOR}:`],
    ['a short pubkey', `10032:${'a'.repeat(63)}:`],
    ['an uppercase pubkey', `10032:${'A'.repeat(64)}:`],
  ])('rejects %s', (_label, value) => {
    expect(parseAddressCoordinate(value)).toBeUndefined();
  });
});

describe('parseDeletionTargets', () => {
  it('collects e and a tags', () => {
    const targets = parseDeletionTargets({
      tags: [
        ['e', EVENT_ID],
        ['a', `10032:${AUTHOR}:`],
        ['k', '10032'],
      ],
    });
    expect(targets.ids).toEqual([EVENT_ID]);
    expect(targets.addresses).toHaveLength(1);
  });

  it('de-duplicates repeated targets', () => {
    const targets = parseDeletionTargets({
      tags: [
        ['e', EVENT_ID],
        ['e', EVENT_ID],
        ['a', `10032:${AUTHOR}:`],
        ['a', `10032:${AUTHOR}:`],
      ],
    });
    expect(targets.ids).toHaveLength(1);
    expect(targets.addresses).toHaveLength(1);
  });

  // One bad tag must not void the good ones alongside it.
  it('skips malformed tags without losing the valid ones', () => {
    const targets = parseDeletionTargets({
      tags: [
        ['e', 'short'],
        ['e', EVENT_ID],
        ['a', 'garbage'],
        ['a', `1:${AUTHOR}:x`],
      ],
    });
    expect(targets.ids).toEqual([EVENT_ID]);
    expect(targets.addresses).toEqual([
      { kind: 1, pubkey: AUTHOR, identifier: 'x' },
    ]);
  });

  it('returns nothing for a request that names nothing', () => {
    expect(parseDeletionTargets({ tags: [] })).toEqual({
      ids: [],
      addresses: [],
    });
  });
});

describe('isDeletableBy -- the NIP-09 authorization rule', () => {
  it('allows an author to retract their own earlier event', () => {
    expect(
      isDeletableBy(
        { pubkey: AUTHOR, created_at: 100 },
        { pubkey: AUTHOR, created_at: 200 }
      )
    ).toBe(true);
  });

  it('allows retracting an event created in the same second', () => {
    expect(
      isDeletableBy(
        { pubkey: AUTHOR, created_at: 200 },
        { pubkey: AUTHOR, created_at: 200 }
      )
    ).toBe(true);
  });

  // The entire trust boundary of NIP-09.
  it('refuses to retract another key’s event', () => {
    expect(
      isDeletableBy(
        { pubkey: STRANGER, created_at: 100 },
        { pubkey: AUTHOR, created_at: 200 }
      )
    ).toBe(false);
  });

  // Without this, one kind:5 would permanently silence every later event the
  // key publishes.
  it('cannot pre-emptively retract a future event', () => {
    expect(
      isDeletableBy(
        { pubkey: AUTHOR, created_at: 300 },
        { pubkey: AUTHOR, created_at: 200 }
      )
    ).toBe(false);
  });
});
