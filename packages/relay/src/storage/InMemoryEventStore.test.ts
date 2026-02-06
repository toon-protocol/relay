import { describe, it, expect, beforeEach } from 'vitest';
import type { NostrEvent } from 'nostr-tools/pure';
import { InMemoryEventStore } from './InMemoryEventStore.js';

function createMockEvent(overrides: Partial<NostrEvent> = {}): NostrEvent {
  return {
    id: 'event1',
    pubkey: 'pubkey1',
    created_at: 1000,
    kind: 1,
    tags: [],
    content: 'test content',
    sig: 'sig1',
    ...overrides,
  };
}

describe('InMemoryEventStore', () => {
  let store: InMemoryEventStore;

  beforeEach(() => {
    store = new InMemoryEventStore();
  });

  describe('store and get', () => {
    it('should store and retrieve events by id', () => {
      const event = createMockEvent({ id: 'abc123' });
      store.store(event);

      const retrieved = store.get('abc123');
      expect(retrieved).toEqual(event);
    });

    it('should return undefined for non-existent event', () => {
      const retrieved = store.get('nonexistent');
      expect(retrieved).toBeUndefined();
    });

    it('should overwrite events with same id', () => {
      const event1 = createMockEvent({ id: 'abc', content: 'first' });
      const event2 = createMockEvent({ id: 'abc', content: 'second' });

      store.store(event1);
      store.store(event2);

      const retrieved = store.get('abc');
      expect(retrieved?.content).toBe('second');
    });
  });

  describe('query', () => {
    beforeEach(() => {
      // Add some test events
      store.store(
        createMockEvent({
          id: 'event1',
          pubkey: 'alice',
          kind: 1,
          created_at: 1000,
          tags: [['e', 'ref1']],
        })
      );
      store.store(
        createMockEvent({
          id: 'event2',
          pubkey: 'bob',
          kind: 1,
          created_at: 2000,
          tags: [['p', 'alice']],
        })
      );
      store.store(
        createMockEvent({
          id: 'event3',
          pubkey: 'alice',
          kind: 4,
          created_at: 3000,
          tags: [],
        })
      );
      store.store(
        createMockEvent({
          id: 'event4',
          pubkey: 'charlie',
          kind: 1,
          created_at: 4000,
          tags: [],
        })
      );
    });

    it('should return all events with empty filters array', () => {
      const results = store.query([]);
      expect(results).toHaveLength(4);
    });

    it('should return all events with empty filter object', () => {
      const results = store.query([{}]);
      expect(results).toHaveLength(4);
    });

    it('should filter by kind', () => {
      const results = store.query([{ kinds: [1] }]);
      expect(results).toHaveLength(3);
      expect(results.every((e) => e.kind === 1)).toBe(true);
    });

    it('should filter by multiple kinds', () => {
      const results = store.query([{ kinds: [1, 4] }]);
      expect(results).toHaveLength(4);
    });

    it('should filter by author', () => {
      const results = store.query([{ authors: ['alice'] }]);
      expect(results).toHaveLength(2);
      expect(results.every((e) => e.pubkey === 'alice')).toBe(true);
    });

    it('should filter by since timestamp', () => {
      const results = store.query([{ since: 2500 }]);
      expect(results).toHaveLength(2);
      expect(results.every((e) => e.created_at >= 2500)).toBe(true);
    });

    it('should filter by until timestamp', () => {
      const results = store.query([{ until: 2000 }]);
      expect(results).toHaveLength(2);
      expect(results.every((e) => e.created_at <= 2000)).toBe(true);
    });

    it('should apply limit to results', () => {
      const results = store.query([{ limit: 2 }]);
      expect(results).toHaveLength(2);
    });

    it('should return events sorted by created_at desc', () => {
      const results = store.query([]);
      expect(results[0].id).toBe('event4');
      expect(results[1].id).toBe('event3');
      expect(results[2].id).toBe('event2');
      expect(results[3].id).toBe('event1');
    });

    it('should return events sorted by created_at desc with limit', () => {
      const results = store.query([{ limit: 2 }]);
      expect(results[0].id).toBe('event4');
      expect(results[1].id).toBe('event3');
    });

    it('should combine filters with AND logic', () => {
      const results = store.query([{ kinds: [1], authors: ['alice'] }]);
      expect(results).toHaveLength(1);
      expect(results[0].id).toBe('event1');
    });

    it('should combine multiple filter objects with OR logic', () => {
      const results = store.query([
        { authors: ['alice'] },
        { authors: ['charlie'] },
      ]);
      expect(results).toHaveLength(3);
    });
  });
});
