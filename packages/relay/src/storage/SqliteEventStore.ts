import Database from 'better-sqlite3';
import type { NostrEvent } from 'nostr-tools/pure';
import type { Filter } from 'nostr-tools/filter';
import type { EventStore } from './InMemoryEventStore.js';

/**
 * SQL schema for the events table.
 */
const SCHEMA_SQL = `
CREATE TABLE IF NOT EXISTS events (
  id TEXT PRIMARY KEY,
  pubkey TEXT NOT NULL,
  kind INTEGER NOT NULL,
  content TEXT NOT NULL,
  tags TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  sig TEXT NOT NULL,
  received_at INTEGER NOT NULL
)
`;

/**
 * SQL for creating indexes on the events table.
 */
const INDEX_SQL = [
  'CREATE INDEX IF NOT EXISTS idx_events_pubkey ON events(pubkey)',
  'CREATE INDEX IF NOT EXISTS idx_events_kind ON events(kind)',
  'CREATE INDEX IF NOT EXISTS idx_events_created_at ON events(created_at)',
  'CREATE INDEX IF NOT EXISTS idx_events_pubkey_kind ON events(pubkey, kind)',
];

/**
 * Initialize the database schema.
 */
function initializeSchema(db: Database.Database): void {
  db.exec(SCHEMA_SQL);
  for (const indexSql of INDEX_SQL) {
    db.exec(indexSql);
  }
}

/**
 * Custom error class for relay storage errors.
 */
export class RelayError extends Error {
  constructor(
    message: string,
    public code: string
  ) {
    super(message);
    this.name = 'RelayError';
  }
}

/**
 * Check if an event kind is in the replaceable range (10000-19999).
 */
function isReplaceableKind(kind: number): boolean {
  return kind >= 10000 && kind <= 19999;
}

/**
 * Check if an event kind is in the parameterized replaceable range (30000-39999).
 */
function isParameterizedReplaceableKind(kind: number): boolean {
  return kind >= 30000 && kind <= 39999;
}

/**
 * Get the 'd' tag value from an event's tags array.
 */
function getDTagValue(tags: string[][]): string {
  const dTag = tags.find((tag) => tag[0] === 'd');
  return dTag?.[1] ?? '';
}

/**
 * SQLite implementation of EventStore.
 * Persists events to a SQLite database file.
 */
export class SqliteEventStore implements EventStore {
  private db: Database.Database;
  private insertStmt: Database.Statement;
  private getStmt: Database.Statement;
  private deleteByPubkeyKindStmt: Database.Statement;
  private deleteByPubkeyKindDTagStmt: Database.Statement;
  private getByPubkeyKindStmt: Database.Statement;
  private getByPubkeyKindDTagStmt: Database.Statement;

  /**
   * Create a new SqliteEventStore.
   * @param dbPath - Path to the database file. Use ':memory:' for in-memory database.
   */
  constructor(dbPath = ':memory:') {
    try {
      this.db = new Database(dbPath);
      initializeSchema(this.db);

      // Prepare statements for better performance
      this.insertStmt = this.db.prepare(`
        INSERT OR REPLACE INTO events (id, pubkey, kind, content, tags, created_at, sig, received_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?)
      `);

      this.getStmt = this.db.prepare('SELECT * FROM events WHERE id = ?');

      this.deleteByPubkeyKindStmt = this.db.prepare(
        'DELETE FROM events WHERE pubkey = ? AND kind = ?'
      );

      this.deleteByPubkeyKindDTagStmt = this.db.prepare(
        "DELETE FROM events WHERE pubkey = ? AND kind = ? AND json_extract(tags, '$') LIKE ?"
      );

      this.getByPubkeyKindStmt = this.db.prepare(
        'SELECT id, created_at FROM events WHERE pubkey = ? AND kind = ?'
      );

      this.getByPubkeyKindDTagStmt = this.db.prepare(
        "SELECT id, created_at FROM events WHERE pubkey = ? AND kind = ? AND tags LIKE ?"
      );
    } catch (error) {
      throw new RelayError(
        `Failed to initialize database: ${error instanceof Error ? error.message : String(error)}`,
        'STORAGE_ERROR'
      );
    }
  }

  /**
   * Store an event in the database.
   * Handles replaceable and parameterized replaceable events according to NIP-01.
   */
  store(event: NostrEvent): void {
    try {
      const tagsJson = JSON.stringify(event.tags);
      const receivedAt = Math.floor(Date.now() / 1000);

      if (isReplaceableKind(event.kind)) {
        // Replaceable event (10000-19999)
        this.storeReplaceableEvent(event, tagsJson, receivedAt);
      } else if (isParameterizedReplaceableKind(event.kind)) {
        // Parameterized replaceable event (30000-39999)
        this.storeParameterizedReplaceableEvent(event, tagsJson, receivedAt);
      } else {
        // Regular event - INSERT OR IGNORE to handle duplicates
        const insertOrIgnore = this.db.prepare(`
          INSERT OR IGNORE INTO events (id, pubkey, kind, content, tags, created_at, sig, received_at)
          VALUES (?, ?, ?, ?, ?, ?, ?, ?)
        `);
        insertOrIgnore.run(
          event.id,
          event.pubkey,
          event.kind,
          event.content,
          tagsJson,
          event.created_at,
          event.sig,
          receivedAt
        );
      }
    } catch (error) {
      if (error instanceof RelayError) {
        throw error;
      }
      throw new RelayError(
        `Failed to store event: ${error instanceof Error ? error.message : String(error)}`,
        'STORAGE_ERROR'
      );
    }
  }

  /**
   * Store a replaceable event (kinds 10000-19999).
   * Only keeps the latest event per pubkey+kind.
   */
  private storeReplaceableEvent(
    event: NostrEvent,
    tagsJson: string,
    receivedAt: number
  ): void {
    const existing = this.getByPubkeyKindStmt.get(
      event.pubkey,
      event.kind
    ) as { id: string; created_at: number } | undefined;

    if (existing) {
      // Only replace if new event is newer, or same time with lower id
      if (
        event.created_at > existing.created_at ||
        (event.created_at === existing.created_at && event.id < existing.id)
      ) {
        // Use transaction for atomicity
        const transaction = this.db.transaction(() => {
          this.deleteByPubkeyKindStmt.run(event.pubkey, event.kind);
          this.insertStmt.run(
            event.id,
            event.pubkey,
            event.kind,
            event.content,
            tagsJson,
            event.created_at,
            event.sig,
            receivedAt
          );
        });
        transaction();
      }
      // If existing event is newer or same, don't replace
    } else {
      // No existing event, just insert
      this.insertStmt.run(
        event.id,
        event.pubkey,
        event.kind,
        event.content,
        tagsJson,
        event.created_at,
        event.sig,
        receivedAt
      );
    }
  }

  /**
   * Store a parameterized replaceable event (kinds 30000-39999).
   * Only keeps the latest event per pubkey+kind+d-tag.
   */
  private storeParameterizedReplaceableEvent(
    event: NostrEvent,
    tagsJson: string,
    receivedAt: number
  ): void {
    const dTagValue = getDTagValue(event.tags);

    // For empty d-tag value, we need to match events that either:
    // 1. Have ["d", ""] in tags
    // 2. Have no d-tag at all (tags doesn't contain "d" as first element)
    let existing: { id: string; created_at: number } | undefined;

    if (dTagValue === '') {
      // Query for events with same pubkey and kind, then filter in code
      const candidates = this.db
        .prepare(
          'SELECT id, created_at, tags FROM events WHERE pubkey = ? AND kind = ?'
        )
        .all(event.pubkey, event.kind) as {
        id: string;
        created_at: number;
        tags: string;
      }[];

      // Find one with empty or missing d-tag
      for (const candidate of candidates) {
        const candidateTags = JSON.parse(candidate.tags) as string[][];
        const candidateDTagValue = getDTagValue(candidateTags);
        if (candidateDTagValue === '') {
          existing = { id: candidate.id, created_at: candidate.created_at };
          break;
        }
      }
    } else {
      const dTagPattern = `%["d","${dTagValue}"%`;
      existing = this.getByPubkeyKindDTagStmt.get(
        event.pubkey,
        event.kind,
        dTagPattern
      ) as { id: string; created_at: number } | undefined;
    }

    if (existing) {
      // Only replace if new event is newer, or same time with lower id
      if (
        event.created_at > existing.created_at ||
        (event.created_at === existing.created_at && event.id < existing.id)
      ) {
        // Use transaction for atomicity - delete by ID for safety
        const transaction = this.db.transaction(() => {
          this.db.prepare('DELETE FROM events WHERE id = ?').run(existing.id);
          this.insertStmt.run(
            event.id,
            event.pubkey,
            event.kind,
            event.content,
            tagsJson,
            event.created_at,
            event.sig,
            receivedAt
          );
        });
        transaction();
      }
      // If existing event is newer or same, don't replace
    } else {
      // No existing event, just insert
      this.insertStmt.run(
        event.id,
        event.pubkey,
        event.kind,
        event.content,
        tagsJson,
        event.created_at,
        event.sig,
        receivedAt
      );
    }
  }

  /**
   * Retrieve an event by its ID.
   */
  get(id: string): NostrEvent | undefined {
    try {
      const row = this.getStmt.get(id) as
        | {
            id: string;
            pubkey: string;
            kind: number;
            content: string;
            tags: string;
            created_at: number;
            sig: string;
          }
        | undefined;

      if (!row) {
        return undefined;
      }

      return {
        id: row.id,
        pubkey: row.pubkey,
        kind: row.kind,
        content: row.content,
        tags: JSON.parse(row.tags) as string[][],
        created_at: row.created_at,
        sig: row.sig,
      };
    } catch (error) {
      throw new RelayError(
        `Failed to get event: ${error instanceof Error ? error.message : String(error)}`,
        'STORAGE_ERROR'
      );
    }
  }

  /**
   * Query events matching any of the provided filters.
   */
  query(filters: Filter[]): NostrEvent[] {
    try {
      const { sql, params } = this.buildQuerySql(filters);
      const stmt = this.db.prepare(sql);
      const rows = stmt.all(...params) as {
        id: string;
        pubkey: string;
        kind: number;
        content: string;
        tags: string;
        created_at: number;
        sig: string;
      }[];

      return rows.map((row) => ({
        id: row.id,
        pubkey: row.pubkey,
        kind: row.kind,
        content: row.content,
        tags: JSON.parse(row.tags) as string[][],
        created_at: row.created_at,
        sig: row.sig,
      }));
    } catch (error) {
      throw new RelayError(
        `Failed to query events: ${error instanceof Error ? error.message : String(error)}`,
        'STORAGE_ERROR'
      );
    }
  }

  /**
   * Build SQL query from filters.
   */
  private buildQuerySql(filters: Filter[]): { sql: string; params: unknown[] } {
    if (filters.length === 0) {
      return { sql: 'SELECT * FROM events ORDER BY created_at DESC', params: [] };
    }

    const conditions: string[] = [];
    const params: unknown[] = [];

    for (const filter of filters) {
      const filterConditions: string[] = [];

      if (filter.ids?.length) {
        // Prefix matching with LIKE
        const idConditions = filter.ids.map(() => 'id LIKE ?');
        filterConditions.push(`(${idConditions.join(' OR ')})`);
        params.push(...filter.ids.map((id) => `${id}%`));
      }

      if (filter.authors?.length) {
        const authorConditions = filter.authors.map(() => 'pubkey LIKE ?');
        filterConditions.push(`(${authorConditions.join(' OR ')})`);
        params.push(...filter.authors.map((a) => `${a}%`));
      }

      if (filter.kinds?.length) {
        filterConditions.push(
          `kind IN (${filter.kinds.map(() => '?').join(', ')})`
        );
        params.push(...filter.kinds);
      }

      if (filter.since !== undefined) {
        filterConditions.push('created_at >= ?');
        params.push(filter.since);
      }

      if (filter.until !== undefined) {
        filterConditions.push('created_at <= ?');
        params.push(filter.until);
      }

      // Handle tag filters (#e, #p, etc.)
      for (const [key, values] of Object.entries(filter)) {
        if (key.startsWith('#') && Array.isArray(values) && values.length > 0) {
          const tagName = key.slice(1);
          const tagConditions = values.map(() => `tags LIKE ?`);
          filterConditions.push(`(${tagConditions.join(' OR ')})`);
          params.push(...values.map((v) => `%["${tagName}","${v}"%`));
        }
      }

      if (filterConditions.length > 0) {
        conditions.push(`(${filterConditions.join(' AND ')})`);
      }
    }

    let sql = 'SELECT * FROM events';
    if (conditions.length > 0) {
      sql += ` WHERE ${conditions.join(' OR ')}`;
    }
    sql += ' ORDER BY created_at DESC';

    // Apply limit from first filter that specifies it
    const limitFilter = filters.find((f) => f.limit !== undefined);
    if (limitFilter?.limit !== undefined) {
      sql += ' LIMIT ?';
      params.push(limitFilter.limit);
    }

    return { sql, params };
  }

  /**
   * Close the database connection.
   */
  close(): void {
    this.db.close();
  }
}
