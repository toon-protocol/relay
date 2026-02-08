import { describe, it, expect, afterEach } from 'vitest';
import { existsSync, unlinkSync } from 'node:fs';
import Database from 'better-sqlite3';
import { SqliteEventStore } from './SqliteEventStore.js';
import type { NostrEvent } from 'nostr-tools/pure';

const TEST_DB_PATH = './test-persistence-events.db';

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

describe('SqliteEventStore persistence', () => {
  afterEach(() => {
    if (existsSync(TEST_DB_PATH)) {
      unlinkSync(TEST_DB_PATH);
    }
  });

  it('should persist events across close and reopen', () => {
    const event = createTestEvent({
      id: 'persist1'.padEnd(64, '0'),
      content: 'persisted content',
    });

    // Store event and close
    const store1 = new SqliteEventStore(TEST_DB_PATH);
    store1.store(event);
    store1.close();

    // Reopen and verify
    const store2 = new SqliteEventStore(TEST_DB_PATH);
    const retrieved = store2.get(event.id);
    store2.close();

    expect(retrieved).toEqual(event);
  });

  it('should persist events retrievable via query() after restart', () => {
    const event = createTestEvent({
      id: 'query1'.padEnd(64, '0'),
      kind: 42,
      created_at: 1700000000,
    });

    const store1 = new SqliteEventStore(TEST_DB_PATH);
    store1.store(event);
    store1.close();

    const store2 = new SqliteEventStore(TEST_DB_PATH);
    const results = store2.query([{ kinds: [42] }]);
    store2.close();

    expect(results).toHaveLength(1);
    expect(results[0]).toEqual(event);
  });

  it('should persist multiple events across restart', () => {
    const events = [
      createTestEvent({ id: 'multi1'.padEnd(64, '0'), created_at: 1000 }),
      createTestEvent({ id: 'multi2'.padEnd(64, '0'), created_at: 2000 }),
      createTestEvent({ id: 'multi3'.padEnd(64, '0'), created_at: 3000 }),
    ];

    const store1 = new SqliteEventStore(TEST_DB_PATH);
    for (const event of events) {
      store1.store(event);
    }
    store1.close();

    const store2 = new SqliteEventStore(TEST_DB_PATH);
    for (const event of events) {
      expect(store2.get(event.id)).toEqual(event);
    }
    const allEvents = store2.query([]);
    store2.close();

    expect(allEvents).toHaveLength(3);
  });

  it('should maintain replaceable event semantics across restarts', () => {
    const pubkey = 'samepubkey'.padEnd(64, '0');

    // Store first replaceable event
    const event1 = createTestEvent({
      id: 'replace1'.padEnd(64, '0'),
      pubkey,
      kind: 10032,
      created_at: 1000,
      content: 'old peer info',
    });

    const store1 = new SqliteEventStore(TEST_DB_PATH);
    store1.store(event1);
    store1.close();

    // Reopen and store newer replaceable event
    const event2 = createTestEvent({
      id: 'replace2'.padEnd(64, '0'),
      pubkey,
      kind: 10032,
      created_at: 2000,
      content: 'new peer info',
    });

    const store2 = new SqliteEventStore(TEST_DB_PATH);
    store2.store(event2);

    // Only the newer event should exist
    expect(store2.get(event1.id)).toBeUndefined();
    expect(store2.get(event2.id)).toEqual(event2);
    store2.close();
  });

  it('should preserve database schema (tables and indexes) across restarts', () => {
    // Create store to initialize schema, then close
    const store1 = new SqliteEventStore(TEST_DB_PATH);
    store1.close();

    // Open raw database and verify schema
    const db = new Database(TEST_DB_PATH);

    // Verify events table exists
    const tables = db
      .prepare(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='events'"
      )
      .all() as { name: string }[];
    expect(tables).toHaveLength(1);
    expect(tables[0].name).toBe('events');

    // Verify all indexes exist
    const indexes = db
      .prepare(
        "SELECT name FROM sqlite_master WHERE type='index' AND tbl_name='events'"
      )
      .all() as { name: string }[];
    const indexNames = indexes.map((i) => i.name);

    expect(indexNames).toContain('idx_events_pubkey');
    expect(indexNames).toContain('idx_events_kind');
    expect(indexNames).toContain('idx_events_created_at');
    expect(indexNames).toContain('idx_events_pubkey_kind');

    db.close();

    // Reopen via SqliteEventStore — should not error (CREATE IF NOT EXISTS)
    const store2 = new SqliteEventStore(TEST_DB_PATH);
    store2.close();
  });
});
