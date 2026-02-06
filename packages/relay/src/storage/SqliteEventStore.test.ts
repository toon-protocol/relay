import { describe, it, expect, beforeEach, afterEach } from 'vitest';
import { existsSync, unlinkSync } from 'fs';
import { SqliteEventStore, RelayError } from './SqliteEventStore.js';
import type { NostrEvent } from 'nostr-tools/pure';
import Database from 'better-sqlite3';

const TEST_DB_PATH = './test-events.db';

/**
 * Helper to create test events with sensible defaults.
 */
function createTestEvent(overrides: Partial<NostrEvent> = {}): NostrEvent {
  return {
    id: 'abc123def456789012345678901234567890123456789012345678901234',
    pubkey: 'pubkey123456789012345678901234567890123456789012345678901234',
    kind: 1,
    content: 'test content',
    tags: [],
    created_at: Math.floor(Date.now() / 1000),
    sig: 'sig1234567890123456789012345678901234567890123456789012345678901234',
    ...overrides,
  };
}

describe('SqliteEventStore', () => {
  let store: SqliteEventStore;

  afterEach(() => {
    if (store) {
      store.close();
    }
    if (existsSync(TEST_DB_PATH)) {
      unlinkSync(TEST_DB_PATH);
    }
  });

  describe('Schema Initialization', () => {
    it('should create events table on construction', () => {
      store = new SqliteEventStore(TEST_DB_PATH);
      const db = new Database(TEST_DB_PATH);
      const tables = db
        .prepare(
          "SELECT name FROM sqlite_master WHERE type='table' AND name='events'"
        )
        .all() as { name: string }[];
      db.close();
      expect(tables).toHaveLength(1);
      expect(tables[0].name).toBe('events');
    });

    it('should create all indexes', () => {
      store = new SqliteEventStore(TEST_DB_PATH);
      const db = new Database(TEST_DB_PATH);
      const indexes = db
        .prepare(
          "SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='events'"
        )
        .all() as { name: string }[];
      db.close();

      const indexNames = indexes.map((i) => i.name);
      expect(indexNames).toContain('idx_events_pubkey');
      expect(indexNames).toContain('idx_events_kind');
      expect(indexNames).toContain('idx_events_created_at');
      expect(indexNames).toContain('idx_events_pubkey_kind');
    });

    it('should create file at specified dbPath', () => {
      store = new SqliteEventStore(TEST_DB_PATH);
      expect(existsSync(TEST_DB_PATH)).toBe(true);
    });

    it('should create in-memory database with :memory: path', () => {
      store = new SqliteEventStore(':memory:');
      // If we get here without error, in-memory database was created
      expect(store).toBeDefined();
    });

    it('should use :memory: as default path', () => {
      store = new SqliteEventStore();
      // If we get here without error, default in-memory database was created
      expect(store).toBeDefined();
    });
  });

  describe('Store and Get Operations', () => {
    beforeEach(() => {
      store = new SqliteEventStore(':memory:');
    });

    it('should store and retrieve an event by ID', () => {
      const event = createTestEvent();
      store.store(event);
      const retrieved = store.get(event.id);
      expect(retrieved).toEqual(event);
    });

    it('should store multiple events', () => {
      const event1 = createTestEvent({ id: 'id1'.padEnd(64, '0') });
      const event2 = createTestEvent({ id: 'id2'.padEnd(64, '0') });
      const event3 = createTestEvent({ id: 'id3'.padEnd(64, '0') });

      store.store(event1);
      store.store(event2);
      store.store(event3);

      expect(store.get(event1.id)).toEqual(event1);
      expect(store.get(event2.id)).toEqual(event2);
      expect(store.get(event3.id)).toEqual(event3);
    });

    it('should return undefined for non-existent ID', () => {
      const result = store.get('nonexistent'.padEnd(64, '0'));
      expect(result).toBeUndefined();
    });

    it('should correctly persist all event fields', () => {
      const event = createTestEvent({
        id: 'testid'.padEnd(64, 'a'),
        pubkey: 'testpubkey'.padEnd(64, 'b'),
        kind: 42,
        content: 'Hello, world!',
        tags: [
          ['e', 'event123'],
          ['p', 'pubkey456'],
        ],
        created_at: 1700000000,
        sig: 'testsig'.padEnd(128, 'c'),
      });

      store.store(event);
      const retrieved = store.get(event.id);

      expect(retrieved?.id).toBe(event.id);
      expect(retrieved?.pubkey).toBe(event.pubkey);
      expect(retrieved?.kind).toBe(42);
      expect(retrieved?.content).toBe('Hello, world!');
      expect(retrieved?.created_at).toBe(1700000000);
      expect(retrieved?.sig).toBe(event.sig);
    });

    it('should correctly serialize and deserialize tags array', () => {
      const event = createTestEvent({
        tags: [
          ['e', 'abc123', 'wss://relay.example.com'],
          ['p', 'def456'],
          ['t', 'nostr'],
          ['d', 'identifier'],
        ],
      });

      store.store(event);
      const retrieved = store.get(event.id);

      expect(retrieved?.tags).toEqual(event.tags);
      expect(retrieved?.tags).toHaveLength(4);
      expect(retrieved?.tags[0]).toEqual([
        'e',
        'abc123',
        'wss://relay.example.com',
      ]);
    });

    it('should handle empty tags array', () => {
      const event = createTestEvent({ tags: [] });
      store.store(event);
      const retrieved = store.get(event.id);
      expect(retrieved?.tags).toEqual([]);
    });

    it('should handle duplicate events gracefully (INSERT OR IGNORE)', () => {
      const event = createTestEvent({ kind: 1 }); // Regular event
      store.store(event);
      store.store(event); // Store same event again
      const retrieved = store.get(event.id);
      expect(retrieved).toEqual(event);
    });
  });

  describe('Replaceable Events (kinds 10000-19999)', () => {
    beforeEach(() => {
      store = new SqliteEventStore(':memory:');
    });

    it('should replace previous event with same pubkey and kind 10000', () => {
      const pubkey = 'samepubkey'.padEnd(64, '0');
      const event1 = createTestEvent({
        id: 'event1'.padEnd(64, '0'),
        pubkey,
        kind: 10000,
        created_at: 1000,
        content: 'old content',
      });
      const event2 = createTestEvent({
        id: 'event2'.padEnd(64, '0'),
        pubkey,
        kind: 10000,
        created_at: 2000,
        content: 'new content',
      });

      store.store(event1);
      store.store(event2);

      // Old event should be gone
      expect(store.get(event1.id)).toBeUndefined();
      // New event should exist
      expect(store.get(event2.id)).toEqual(event2);
    });

    it('should replace previous event with kind 10032 (ILP_PEER_INFO)', () => {
      const pubkey = 'samepubkey'.padEnd(64, '0');
      const event1 = createTestEvent({
        id: 'event1'.padEnd(64, '0'),
        pubkey,
        kind: 10032,
        created_at: 1000,
      });
      const event2 = createTestEvent({
        id: 'event2'.padEnd(64, '0'),
        pubkey,
        kind: 10032,
        created_at: 2000,
      });

      store.store(event1);
      store.store(event2);

      expect(store.get(event1.id)).toBeUndefined();
      expect(store.get(event2.id)).toEqual(event2);
    });

    it('should replace previous event with kind 15000', () => {
      const pubkey = 'samepubkey'.padEnd(64, '0');
      const event1 = createTestEvent({
        id: 'event1'.padEnd(64, '0'),
        pubkey,
        kind: 15000,
        created_at: 1000,
      });
      const event2 = createTestEvent({
        id: 'event2'.padEnd(64, '0'),
        pubkey,
        kind: 15000,
        created_at: 2000,
      });

      store.store(event1);
      store.store(event2);

      expect(store.get(event1.id)).toBeUndefined();
      expect(store.get(event2.id)).toEqual(event2);
    });

    it('should NOT replace event from different pubkey', () => {
      const event1 = createTestEvent({
        id: 'event1'.padEnd(64, '0'),
        pubkey: 'pubkey1'.padEnd(64, '0'),
        kind: 10000,
        created_at: 1000,
      });
      const event2 = createTestEvent({
        id: 'event2'.padEnd(64, '0'),
        pubkey: 'pubkey2'.padEnd(64, '0'),
        kind: 10000,
        created_at: 2000,
      });

      store.store(event1);
      store.store(event2);

      // Both events should exist
      expect(store.get(event1.id)).toEqual(event1);
      expect(store.get(event2.id)).toEqual(event2);
    });

    it('should NOT replace newer event with older event', () => {
      const pubkey = 'samepubkey'.padEnd(64, '0');
      const newerEvent = createTestEvent({
        id: 'newer'.padEnd(64, '0'),
        pubkey,
        kind: 10000,
        created_at: 2000,
        content: 'newer',
      });
      const olderEvent = createTestEvent({
        id: 'older'.padEnd(64, '0'),
        pubkey,
        kind: 10000,
        created_at: 1000,
        content: 'older',
      });

      store.store(newerEvent);
      store.store(olderEvent); // This should be ignored

      expect(store.get(newerEvent.id)).toEqual(newerEvent);
      expect(store.get(olderEvent.id)).toBeUndefined();
    });

    it('should use lexicographic ID comparison when created_at is equal', () => {
      const pubkey = 'samepubkey'.padEnd(64, '0');
      const eventA = createTestEvent({
        id: 'aaaa'.padEnd(64, '0'), // Lower lexicographically
        pubkey,
        kind: 10000,
        created_at: 1000,
      });
      const eventB = createTestEvent({
        id: 'bbbb'.padEnd(64, '0'), // Higher lexicographically
        pubkey,
        kind: 10000,
        created_at: 1000, // Same timestamp
      });

      // Store higher ID first
      store.store(eventB);
      // Store lower ID - should replace because lower ID wins on tie
      store.store(eventA);

      expect(store.get(eventA.id)).toEqual(eventA);
      expect(store.get(eventB.id)).toBeUndefined();
    });
  });

  describe('Regular Events (kind < 10000)', () => {
    beforeEach(() => {
      store = new SqliteEventStore(':memory:');
    });

    it('should NOT replace events for kind 1', () => {
      const pubkey = 'samepubkey'.padEnd(64, '0');
      const event1 = createTestEvent({
        id: 'event1'.padEnd(64, '0'),
        pubkey,
        kind: 1,
        created_at: 1000,
      });
      const event2 = createTestEvent({
        id: 'event2'.padEnd(64, '0'),
        pubkey,
        kind: 1,
        created_at: 2000,
      });

      store.store(event1);
      store.store(event2);

      // Both events should exist
      expect(store.get(event1.id)).toEqual(event1);
      expect(store.get(event2.id)).toEqual(event2);
    });
  });

  describe('Parameterized Replaceable Events (kinds 30000-39999)', () => {
    beforeEach(() => {
      store = new SqliteEventStore(':memory:');
    });

    it('should replace event with same pubkey, kind, and d-tag', () => {
      const pubkey = 'samepubkey'.padEnd(64, '0');
      const event1 = createTestEvent({
        id: 'event1'.padEnd(64, '0'),
        pubkey,
        kind: 30000,
        created_at: 1000,
        tags: [['d', 'my-identifier']],
        content: 'old',
      });
      const event2 = createTestEvent({
        id: 'event2'.padEnd(64, '0'),
        pubkey,
        kind: 30000,
        created_at: 2000,
        tags: [['d', 'my-identifier']],
        content: 'new',
      });

      store.store(event1);
      store.store(event2);

      expect(store.get(event1.id)).toBeUndefined();
      expect(store.get(event2.id)).toEqual(event2);
    });

    it('should NOT replace event with different d-tag', () => {
      const pubkey = 'samepubkey'.padEnd(64, '0');
      const event1 = createTestEvent({
        id: 'event1'.padEnd(64, '0'),
        pubkey,
        kind: 30000,
        created_at: 1000,
        tags: [['d', 'identifier-1']],
      });
      const event2 = createTestEvent({
        id: 'event2'.padEnd(64, '0'),
        pubkey,
        kind: 30000,
        created_at: 2000,
        tags: [['d', 'identifier-2']],
      });

      store.store(event1);
      store.store(event2);

      // Both should exist
      expect(store.get(event1.id)).toEqual(event1);
      expect(store.get(event2.id)).toEqual(event2);
    });

    it('should handle empty d-tag value', () => {
      const pubkey = 'samepubkey'.padEnd(64, '0');
      const event1 = createTestEvent({
        id: 'event1'.padEnd(64, '0'),
        pubkey,
        kind: 30000,
        created_at: 1000,
        tags: [['d', '']],
      });
      const event2 = createTestEvent({
        id: 'event2'.padEnd(64, '0'),
        pubkey,
        kind: 30000,
        created_at: 2000,
        tags: [['d', '']],
      });

      store.store(event1);
      store.store(event2);

      expect(store.get(event1.id)).toBeUndefined();
      expect(store.get(event2.id)).toEqual(event2);
    });

    it('should treat missing d-tag as empty string', () => {
      const pubkey = 'samepubkey'.padEnd(64, '0');
      const event1 = createTestEvent({
        id: 'event1'.padEnd(64, '0'),
        pubkey,
        kind: 30000,
        created_at: 1000,
        tags: [], // No d-tag
      });
      const event2 = createTestEvent({
        id: 'event2'.padEnd(64, '0'),
        pubkey,
        kind: 30000,
        created_at: 2000,
        tags: [['d', '']], // Empty d-tag
      });

      store.store(event1);
      store.store(event2);

      // Should replace because both have effectively empty d-tag
      expect(store.get(event1.id)).toBeUndefined();
      expect(store.get(event2.id)).toEqual(event2);
    });
  });

  describe('Query Operations', () => {
    beforeEach(() => {
      store = new SqliteEventStore(':memory:');
    });

    it('should return all events with empty filters', () => {
      const event1 = createTestEvent({
        id: 'id1'.padEnd(64, '0'),
        created_at: 1000,
      });
      const event2 = createTestEvent({
        id: 'id2'.padEnd(64, '0'),
        created_at: 2000,
      });
      const event3 = createTestEvent({
        id: 'id3'.padEnd(64, '0'),
        created_at: 3000,
      });

      store.store(event1);
      store.store(event2);
      store.store(event3);

      const results = store.query([]);
      expect(results).toHaveLength(3);
    });

    it('should filter by ids (exact match)', () => {
      const event1 = createTestEvent({ id: 'abc123'.padEnd(64, '0') });
      const event2 = createTestEvent({ id: 'def456'.padEnd(64, '0') });

      store.store(event1);
      store.store(event2);

      const results = store.query([{ ids: [event1.id] }]);
      expect(results).toHaveLength(1);
      expect(results[0]).toEqual(event1);
    });

    it('should filter by ids (prefix match)', () => {
      const event1 = createTestEvent({ id: 'abc123'.padEnd(64, '0') });
      const event2 = createTestEvent({ id: 'abc456'.padEnd(64, '0') });
      const event3 = createTestEvent({ id: 'def789'.padEnd(64, '0') });

      store.store(event1);
      store.store(event2);
      store.store(event3);

      const results = store.query([{ ids: ['abc'] }]);
      expect(results).toHaveLength(2);
      expect(results.map((e) => e.id)).toContain(event1.id);
      expect(results.map((e) => e.id)).toContain(event2.id);
    });

    it('should filter by authors', () => {
      const event1 = createTestEvent({ pubkey: 'author1'.padEnd(64, '0') });
      const event2 = createTestEvent({
        id: 'other'.padEnd(64, '0'),
        pubkey: 'author2'.padEnd(64, '0'),
      });

      store.store(event1);
      store.store(event2);

      const results = store.query([{ authors: ['author1'.padEnd(64, '0')] }]);
      expect(results).toHaveLength(1);
      expect(results[0]).toEqual(event1);
    });

    it('should filter by kinds', () => {
      const event1 = createTestEvent({
        id: 'id1'.padEnd(64, '0'),
        kind: 1,
      });
      const event2 = createTestEvent({
        id: 'id2'.padEnd(64, '0'),
        kind: 3,
      });
      const event3 = createTestEvent({
        id: 'id3'.padEnd(64, '0'),
        kind: 1,
      });

      store.store(event1);
      store.store(event2);
      store.store(event3);

      const results = store.query([{ kinds: [1] }]);
      expect(results).toHaveLength(2);
      expect(results.every((e) => e.kind === 1)).toBe(true);
    });

    it('should filter by multiple kinds', () => {
      const event1 = createTestEvent({
        id: 'id1'.padEnd(64, '0'),
        kind: 1,
      });
      const event2 = createTestEvent({
        id: 'id2'.padEnd(64, '0'),
        kind: 3,
      });
      const event3 = createTestEvent({
        id: 'id3'.padEnd(64, '0'),
        kind: 7,
      });

      store.store(event1);
      store.store(event2);
      store.store(event3);

      const results = store.query([{ kinds: [1, 3] }]);
      expect(results).toHaveLength(2);
      expect(results.map((e) => e.kind).sort()).toEqual([1, 3]);
    });

    it('should filter by since', () => {
      const event1 = createTestEvent({
        id: 'id1'.padEnd(64, '0'),
        created_at: 1000,
      });
      const event2 = createTestEvent({
        id: 'id2'.padEnd(64, '0'),
        created_at: 2000,
      });
      const event3 = createTestEvent({
        id: 'id3'.padEnd(64, '0'),
        created_at: 3000,
      });

      store.store(event1);
      store.store(event2);
      store.store(event3);

      const results = store.query([{ since: 2000 }]);
      expect(results).toHaveLength(2);
      expect(results.every((e) => e.created_at >= 2000)).toBe(true);
    });

    it('should filter by until', () => {
      const event1 = createTestEvent({
        id: 'id1'.padEnd(64, '0'),
        created_at: 1000,
      });
      const event2 = createTestEvent({
        id: 'id2'.padEnd(64, '0'),
        created_at: 2000,
      });
      const event3 = createTestEvent({
        id: 'id3'.padEnd(64, '0'),
        created_at: 3000,
      });

      store.store(event1);
      store.store(event2);
      store.store(event3);

      const results = store.query([{ until: 2000 }]);
      expect(results).toHaveLength(2);
      expect(results.every((e) => e.created_at <= 2000)).toBe(true);
    });

    it('should apply limit', () => {
      for (let i = 0; i < 10; i++) {
        store.store(
          createTestEvent({
            id: `id${i}`.padEnd(64, '0'),
            created_at: 1000 + i,
          })
        );
      }

      const results = store.query([{ limit: 5 }]);
      expect(results).toHaveLength(5);
    });

    it('should filter by #e tag', () => {
      const event1 = createTestEvent({
        id: 'id1'.padEnd(64, '0'),
        tags: [['e', 'target123']],
      });
      const event2 = createTestEvent({
        id: 'id2'.padEnd(64, '0'),
        tags: [['e', 'other456']],
      });
      const event3 = createTestEvent({
        id: 'id3'.padEnd(64, '0'),
        tags: [],
      });

      store.store(event1);
      store.store(event2);
      store.store(event3);

      const results = store.query([{ '#e': ['target123'] }]);
      expect(results).toHaveLength(1);
      expect(results[0]).toEqual(event1);
    });

    it('should filter by #p tag', () => {
      const event1 = createTestEvent({
        id: 'id1'.padEnd(64, '0'),
        tags: [['p', 'pubkey123']],
      });
      const event2 = createTestEvent({
        id: 'id2'.padEnd(64, '0'),
        tags: [['p', 'pubkey456']],
      });

      store.store(event1);
      store.store(event2);

      const results = store.query([{ '#p': ['pubkey123'] }]);
      expect(results).toHaveLength(1);
      expect(results[0]).toEqual(event1);
    });

    it('should combine multiple filters with OR logic', () => {
      const event1 = createTestEvent({
        id: 'id1'.padEnd(64, '0'),
        kind: 1,
        pubkey: 'author1'.padEnd(64, '0'),
      });
      const event2 = createTestEvent({
        id: 'id2'.padEnd(64, '0'),
        kind: 3,
        pubkey: 'author2'.padEnd(64, '0'),
      });
      const event3 = createTestEvent({
        id: 'id3'.padEnd(64, '0'),
        kind: 7,
        pubkey: 'author3'.padEnd(64, '0'),
      });

      store.store(event1);
      store.store(event2);
      store.store(event3);

      // Match kind 1 OR kind 3
      const results = store.query([{ kinds: [1] }, { kinds: [3] }]);
      expect(results).toHaveLength(2);
      expect(results.map((e) => e.kind).sort()).toEqual([1, 3]);
    });

    it('should sort results by created_at descending', () => {
      const event1 = createTestEvent({
        id: 'id1'.padEnd(64, '0'),
        created_at: 1000,
      });
      const event2 = createTestEvent({
        id: 'id2'.padEnd(64, '0'),
        created_at: 3000,
      });
      const event3 = createTestEvent({
        id: 'id3'.padEnd(64, '0'),
        created_at: 2000,
      });

      store.store(event1);
      store.store(event2);
      store.store(event3);

      const results = store.query([]);
      expect(results[0].created_at).toBe(3000);
      expect(results[1].created_at).toBe(2000);
      expect(results[2].created_at).toBe(1000);
    });
  });

  describe('Close Method', () => {
    it('should close the database connection', () => {
      store = new SqliteEventStore(':memory:');
      const event = createTestEvent();
      store.store(event);

      store.close();

      // After close, operations should throw
      expect(() => store.get(event.id)).toThrow();
    });
  });

  describe('Error Handling', () => {
    it('should throw RelayError with STORAGE_ERROR code on database errors', () => {
      store = new SqliteEventStore(':memory:');
      store.close();

      try {
        store.get('test');
        expect.fail('Should have thrown');
      } catch (error) {
        expect(error).toBeInstanceOf(RelayError);
        expect((error as RelayError).code).toBe('STORAGE_ERROR');
      }
    });
  });
});
