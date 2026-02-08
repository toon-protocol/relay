import { describe, it, expect } from 'vitest';
import type { NostrEvent } from 'nostr-tools/pure';
import type { Filter } from 'nostr-tools/filter';
import { matchFilter } from './matchFilter.js';

function createMockEvent(overrides: Partial<NostrEvent> = {}): NostrEvent {
  return {
    id: 'abcdef123456',
    pubkey: 'pubkey123456',
    created_at: 1000,
    kind: 1,
    tags: [],
    content: 'test content',
    sig: 'sig1',
    ...overrides,
  };
}

describe('matchFilter', () => {
  describe('empty filter', () => {
    it('should match everything with empty filter object', () => {
      const event = createMockEvent();
      expect(matchFilter(event, {})).toBe(true);
    });
  });

  describe('ids filter', () => {
    it('should match with exact id', () => {
      const event = createMockEvent({ id: 'abcdef123456' });
      const filter: Filter = { ids: ['abcdef123456'] };
      expect(matchFilter(event, filter)).toBe(true);
    });

    it('should match with id prefix', () => {
      const event = createMockEvent({ id: 'abcdef123456' });
      const filter: Filter = { ids: ['abc'] };
      expect(matchFilter(event, filter)).toBe(true);
    });

    it('should not match with non-matching id', () => {
      const event = createMockEvent({ id: 'abcdef123456' });
      const filter: Filter = { ids: ['xyz'] };
      expect(matchFilter(event, filter)).toBe(false);
    });

    it('should match if any id matches', () => {
      const event = createMockEvent({ id: 'abcdef123456' });
      const filter: Filter = { ids: ['xyz', 'abc', '123'] };
      expect(matchFilter(event, filter)).toBe(true);
    });
  });

  describe('authors filter', () => {
    it('should match with exact author', () => {
      const event = createMockEvent({ pubkey: 'alice123456' });
      const filter: Filter = { authors: ['alice123456'] };
      expect(matchFilter(event, filter)).toBe(true);
    });

    it('should match with author prefix', () => {
      const event = createMockEvent({ pubkey: 'alice123456' });
      const filter: Filter = { authors: ['alice'] };
      expect(matchFilter(event, filter)).toBe(true);
    });

    it('should not match with non-matching author', () => {
      const event = createMockEvent({ pubkey: 'alice123456' });
      const filter: Filter = { authors: ['bob'] };
      expect(matchFilter(event, filter)).toBe(false);
    });

    it('should match if any author matches', () => {
      const event = createMockEvent({ pubkey: 'alice123456' });
      const filter: Filter = { authors: ['bob', 'alice', 'charlie'] };
      expect(matchFilter(event, filter)).toBe(true);
    });
  });

  describe('kinds filter', () => {
    it('should match with exact kind', () => {
      const event = createMockEvent({ kind: 1 });
      const filter: Filter = { kinds: [1] };
      expect(matchFilter(event, filter)).toBe(true);
    });

    it('should not match with non-matching kind', () => {
      const event = createMockEvent({ kind: 1 });
      const filter: Filter = { kinds: [4] };
      expect(matchFilter(event, filter)).toBe(false);
    });

    it('should match if any kind matches', () => {
      const event = createMockEvent({ kind: 1 });
      const filter: Filter = { kinds: [0, 1, 2] };
      expect(matchFilter(event, filter)).toBe(true);
    });
  });

  describe('since filter', () => {
    it('should match when created_at >= since', () => {
      const event = createMockEvent({ created_at: 1000 });
      const filter: Filter = { since: 1000 };
      expect(matchFilter(event, filter)).toBe(true);
    });

    it('should match when created_at > since', () => {
      const event = createMockEvent({ created_at: 2000 });
      const filter: Filter = { since: 1000 };
      expect(matchFilter(event, filter)).toBe(true);
    });

    it('should not match when created_at < since', () => {
      const event = createMockEvent({ created_at: 500 });
      const filter: Filter = { since: 1000 };
      expect(matchFilter(event, filter)).toBe(false);
    });
  });

  describe('until filter', () => {
    it('should match when created_at <= until', () => {
      const event = createMockEvent({ created_at: 1000 });
      const filter: Filter = { until: 1000 };
      expect(matchFilter(event, filter)).toBe(true);
    });

    it('should match when created_at < until', () => {
      const event = createMockEvent({ created_at: 500 });
      const filter: Filter = { until: 1000 };
      expect(matchFilter(event, filter)).toBe(true);
    });

    it('should not match when created_at > until', () => {
      const event = createMockEvent({ created_at: 2000 });
      const filter: Filter = { until: 1000 };
      expect(matchFilter(event, filter)).toBe(false);
    });
  });

  describe('tag filters', () => {
    it('should match #e tag filter', () => {
      const event = createMockEvent({
        tags: [['e', 'eventid123']],
      });
      const filter: Filter = { '#e': ['eventid123'] };
      expect(matchFilter(event, filter)).toBe(true);
    });

    it('should match #p tag filter', () => {
      const event = createMockEvent({
        tags: [['p', 'pubkey123']],
      });
      const filter: Filter = { '#p': ['pubkey123'] };
      expect(matchFilter(event, filter)).toBe(true);
    });

    it('should not match if tag value not present', () => {
      const event = createMockEvent({
        tags: [['e', 'eventid123']],
      });
      const filter: Filter = { '#e': ['differentid'] };
      expect(matchFilter(event, filter)).toBe(false);
    });

    it('should not match if tag type not present', () => {
      const event = createMockEvent({
        tags: [['e', 'eventid123']],
      });
      const filter: Filter = { '#p': ['eventid123'] };
      expect(matchFilter(event, filter)).toBe(false);
    });

    it('should match if any tag value matches', () => {
      const event = createMockEvent({
        tags: [['e', 'event2']],
      });
      const filter: Filter = { '#e': ['event1', 'event2', 'event3'] };
      expect(matchFilter(event, filter)).toBe(true);
    });

    it('should match with multiple tags of same type', () => {
      const event = createMockEvent({
        tags: [
          ['e', 'event1'],
          ['e', 'event2'],
        ],
      });
      const filter: Filter = { '#e': ['event2'] };
      expect(matchFilter(event, filter)).toBe(true);
    });
  });

  describe('combined filters (AND logic)', () => {
    it('should match when all conditions are met', () => {
      const event = createMockEvent({
        id: 'abc123',
        pubkey: 'alice',
        kind: 1,
        created_at: 1500,
        tags: [['e', 'ref1']],
      });
      const filter: Filter = {
        ids: ['abc'],
        authors: ['alice'],
        kinds: [1],
        since: 1000,
        until: 2000,
        '#e': ['ref1'],
      };
      expect(matchFilter(event, filter)).toBe(true);
    });

    it('should not match when one condition fails', () => {
      const event = createMockEvent({
        id: 'abc123',
        pubkey: 'bob', // Wrong author
        kind: 1,
        created_at: 1500,
      });
      const filter: Filter = {
        ids: ['abc'],
        authors: ['alice'],
        kinds: [1],
      };
      expect(matchFilter(event, filter)).toBe(false);
    });

    it('should match kind and author together', () => {
      const event = createMockEvent({
        pubkey: 'alice',
        kind: 4,
      });
      const filter: Filter = {
        kinds: [4],
        authors: ['alice'],
      };
      expect(matchFilter(event, filter)).toBe(true);
    });

    it('should match since and until range', () => {
      const event = createMockEvent({ created_at: 1500 });
      const filter: Filter = { since: 1000, until: 2000 };
      expect(matchFilter(event, filter)).toBe(true);
    });

    it('should not match outside since/until range', () => {
      const event = createMockEvent({ created_at: 3000 });
      const filter: Filter = { since: 1000, until: 2000 };
      expect(matchFilter(event, filter)).toBe(false);
    });
  });
});
