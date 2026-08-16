import Database from 'better-sqlite3';
import type { NostrEvent } from 'nostr-tools/pure';
import type { Filter } from 'nostr-tools/filter';
import type { EventStore, EventStoreOptions } from './InMemoryEventStore.js';
import { getExpiration } from '../nips/expiration.js';
import {
  isDeletionKind,
  isDeletableBy,
  parseDeletionTargets,
} from '../nips/deletion.js';

/**
 * SQL schema for the events table.
 *
 * `expires_at` is the event's NIP-40 expiration in unix seconds, NULL when
 * the event never expires. It is a denormalized copy of the `expiration` tag
 * so that "do not serve expired events" is a WHERE clause the query planner
 * can use with an index, rather than a JSON parse of every candidate row —
 * serve-time expiry enforcement is on the hot read path.
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
  received_at INTEGER NOT NULL,
  expires_at INTEGER
)
`;

/**
 * NIP-09 tombstones, by event id.
 *
 * A row here means "pubkey P asked us to delete event E". The pubkey is kept
 * so a deletion request can tombstone an id this relay has never seen (a
 * legitimate case: the delete outraces the event, or reaches a relay that
 * never carried it) WITHOUT that becoming a way to pre-block someone else's
 * event — on arrival the event is only refused when its own pubkey matches
 * the pubkey that requested the deletion.
 */
const DELETED_EVENTS_SCHEMA_SQL = `
CREATE TABLE IF NOT EXISTS deleted_events (
  event_id TEXT PRIMARY KEY,
  pubkey TEXT NOT NULL,
  deleted_at INTEGER NOT NULL
)
`;

/**
 * NIP-09 tombstones, by addressable coordinate (`<kind>:<pubkey>:<d>`).
 *
 * `deleted_at` is the deletion request's `created_at`: it retracts matching
 * events at or before that moment and no later ones, so a node can delete its
 * current announce and immediately publish a fresh one.
 */
const DELETED_ADDRESSES_SCHEMA_SQL = `
CREATE TABLE IF NOT EXISTS deleted_addresses (
  coordinate TEXT PRIMARY KEY,
  deleted_at INTEGER NOT NULL
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
  // Partial index: only the small minority of events that expire at all.
  'CREATE INDEX IF NOT EXISTS idx_events_expires_at ON events(expires_at) WHERE expires_at IS NOT NULL',
];

/**
 * Add `expires_at` to a pre-NIP-40 database and backfill it from the stored
 * tags.
 *
 * This runs against LIVE relay databases that already hold every event the
 * node has ever accepted, so the backfill is scoped by `tags LIKE
 * '%"expiration"%'` — on the devnet relay that narrows a table dominated by
 * huddle-frame events down to a handful of announces. It is a one-time cost
 * on the first boot after upgrading; subsequent boots see the column and skip
 * out immediately.
 */
function migrateExpiresAtColumn(db: Database.Database): void {
  const columns = db.prepare('PRAGMA table_info(events)').all() as {
    name: string;
  }[];
  if (columns.some((column) => column.name === 'expires_at')) return;

  db.exec('ALTER TABLE events ADD COLUMN expires_at INTEGER');

  const candidates = db
    .prepare(`SELECT id, tags FROM events WHERE tags LIKE '%"expiration"%'`)
    .all() as { id: string; tags: string }[];

  const update = db.prepare('UPDATE events SET expires_at = ? WHERE id = ?');
  const backfill = db.transaction(() => {
    for (const row of candidates) {
      let tags: string[][];
      try {
        tags = JSON.parse(row.tags) as string[][];
      } catch {
        continue;
      }
      const expiresAt = getExpiration({ tags });
      if (expiresAt !== undefined) update.run(expiresAt, row.id);
    }
  });
  backfill();
}

/**
 * Initialize the database schema.
 */
function initializeSchema(db: Database.Database): void {
  db.exec(SCHEMA_SQL);
  db.exec(DELETED_EVENTS_SCHEMA_SQL);
  db.exec(DELETED_ADDRESSES_SCHEMA_SQL);
  migrateExpiresAtColumn(db);
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
 * Excludes TOON-specific parameterized kinds 10032-10099.
 */
function isReplaceableKind(kind: number): boolean {
  return kind >= 10000 && kind <= 19999 && !(kind >= 10032 && kind <= 10099);
}

/**
 * Check if an event kind is in the parameterized replaceable range.
 * Covers NIP-33 (30000-39999) and TOON-specific kinds (10032-10099).
 */
function isParameterizedReplaceableKind(kind: number): boolean {
  return (kind >= 30000 && kind <= 39999) || (kind >= 10032 && kind <= 10099);
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
  private insertOrIgnoreStmt: Database.Statement;
  private getStmt: Database.Statement;
  private deleteByPubkeyKindStmt: Database.Statement;
  private deleteByPubkeyKindDTagStmt: Database.Statement;
  private getByPubkeyKindStmt: Database.Statement;
  private getByPubkeyKindDTagStmt: Database.Statement;
  private tombstoneIdStmt: Database.Statement;
  private getTombstoneStmt: Database.Statement;
  private tombstoneAddressStmt: Database.Statement;
  private getAddressTombstoneStmt: Database.Statement;
  private deleteExpiredStmt: Database.Statement;
  private readonly enforceExpiration: boolean;
  private readonly blockedEventIds: ReadonlySet<string>;

  /**
   * Create a new SqliteEventStore.
   * @param dbPath - Path to the database file. Use ':memory:' for in-memory database.
   * @param options - Expiry-enforcement and operator-blocklist settings.
   */
  constructor(dbPath = ':memory:', options: EventStoreOptions = {}) {
    this.enforceExpiration = options.enforceExpiration ?? true;
    this.blockedEventIds = new Set(options.blockedEventIds ?? []);
    try {
      this.db = new Database(dbPath);

      // WAL + synchronous=NORMAL (connector#685): the default rollback
      // journal with synchronous=FULL costs two fsyncs per autocommit
      // INSERT -- ~4ms of event-loop blockage per stored event, which was
      // the dominant share of the paid-write pipeline's ~150 events/s
      // global admission ceiling. WAL with synchronous=NORMAL keeps
      // durability at the checkpoint level (an OS crash can lose the last
      // moments of writes, an app crash loses nothing) and turns each
      // insert into a memory-speed WAL append. On a ':memory:' database
      // the pragma is a harmless no-op.
      this.db.pragma('journal_mode = WAL');
      this.db.pragma('synchronous = NORMAL');

      initializeSchema(this.db);

      // Prepare statements for better performance
      this.insertStmt = this.db.prepare(`
        INSERT OR REPLACE INTO events (id, pubkey, kind, content, tags, created_at, sig, received_at, expires_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
      `);

      this.insertOrIgnoreStmt = this.db.prepare(`
        INSERT OR IGNORE INTO events (id, pubkey, kind, content, tags, created_at, sig, received_at, expires_at)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
      `);

      this.getStmt = this.db.prepare('SELECT * FROM events WHERE id = ?');

      this.tombstoneIdStmt = this.db.prepare(
        'INSERT OR REPLACE INTO deleted_events (event_id, pubkey, deleted_at) VALUES (?, ?, ?)'
      );
      this.getTombstoneStmt = this.db.prepare(
        'SELECT pubkey FROM deleted_events WHERE event_id = ?'
      );
      // A later deletion request must not lower an earlier one's watermark,
      // so keep the MAX of the two `created_at` values.
      this.tombstoneAddressStmt = this.db.prepare(
        `INSERT INTO deleted_addresses (coordinate, deleted_at) VALUES (?, ?)
         ON CONFLICT(coordinate) DO UPDATE SET deleted_at = MAX(deleted_at, excluded.deleted_at)`
      );
      this.getAddressTombstoneStmt = this.db.prepare(
        'SELECT deleted_at FROM deleted_addresses WHERE coordinate = ?'
      );
      this.deleteExpiredStmt = this.db.prepare(
        'DELETE FROM events WHERE expires_at IS NOT NULL AND expires_at <= ?'
      );

      // Sweep any operator-blocked ids that this database already holds.
      // Refusing them on write only helps for events that have not arrived
      // yet; the whole point of the blocklist is litter that is ALREADY
      // stored (see nips/blocklist.ts).
      if (this.blockedEventIds.size > 0) {
        const purge = this.db.prepare('DELETE FROM events WHERE id = ?');
        const purgeAll = this.db.transaction(() => {
          for (const id of this.blockedEventIds) purge.run(id);
        });
        purgeAll();
      }

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
        'SELECT id, created_at FROM events WHERE pubkey = ? AND kind = ? AND tags LIKE ?'
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
   *
   * Handles replaceable and parameterized replaceable events according to
   * NIP-01, applies NIP-09 deletion requests, and refuses events the operator
   * has blocked or that a previous NIP-09 request already retracted.
   */
  store(event: NostrEvent): void {
    try {
      // --- Operator blocklist (see nips/blocklist.ts) ---
      if (this.blockedEventIds.has(event.id)) return;

      // --- NIP-09: refuse re-publication of an already-deleted event ---
      if (this.isRetracted(event)) return;

      const tagsJson = JSON.stringify(event.tags);
      const receivedAt = Math.floor(Date.now() / 1000);

      // --- NIP-09: a kind:5 retracts the author's OWN events, then persists
      // itself as an ordinary event so it can propagate to other relays. ---
      if (isDeletionKind(event.kind)) {
        this.applyDeletion(event);
      }

      if (isReplaceableKind(event.kind)) {
        // Replaceable event (10000-19999)
        this.storeReplaceableEvent(event, tagsJson, receivedAt);
      } else if (isParameterizedReplaceableKind(event.kind)) {
        // Parameterized replaceable event (30000-39999)
        this.storeParameterizedReplaceableEvent(event, tagsJson, receivedAt);
      } else {
        // Regular event - INSERT OR IGNORE to handle duplicates. Uses the
        // statement prepared once in the constructor: re-preparing here on
        // every call was measurable overhead on the hot write path.
        this.runInsert(this.insertOrIgnoreStmt, event, tagsJson, receivedAt);
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
    const existing = this.getByPubkeyKindStmt.get(event.pubkey, event.kind) as
      | { id: string; created_at: number }
      | undefined;

    if (existing) {
      // Only replace if new event is newer, or same time with lower id
      if (
        event.created_at > existing.created_at ||
        (event.created_at === existing.created_at && event.id < existing.id)
      ) {
        // Use transaction for atomicity
        const transaction = this.db.transaction(() => {
          this.deleteByPubkeyKindStmt.run(event.pubkey, event.kind);
          this.runInsert(this.insertStmt, event, tagsJson, receivedAt);
        });
        transaction();
      }
      // If existing event is newer or same, don't replace
    } else {
      // No existing event, just insert
      this.runInsert(this.insertStmt, event, tagsJson, receivedAt);
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
          this.runInsert(this.insertStmt, event, tagsJson, receivedAt);
        });
        transaction();
      }
      // If existing event is newer or same, don't replace
    } else {
      // No existing event, just insert
      this.runInsert(this.insertStmt, event, tagsJson, receivedAt);
    }
  }

  /**
   * Bind an event to one of the prepared INSERT statements.
   *
   * The `expires_at` column is derived here, at the single point every write
   * funnels through, so no insert path can forget it.
   */
  private runInsert(
    stmt: Database.Statement,
    event: NostrEvent,
    tagsJson: string,
    receivedAt: number
  ): void {
    stmt.run(
      event.id,
      event.pubkey,
      event.kind,
      event.content,
      tagsJson,
      event.created_at,
      event.sig,
      receivedAt,
      getExpiration(event) ?? null
    );
  }

  /**
   * The NIP-01 addressable coordinate of an event, `<kind>:<pubkey>:<d>`.
   * The `d` value is the empty string for events that carry no `d` tag —
   * which is every kind:10032 announce on the network today.
   */
  private static coordinateOf(event: NostrEvent): string {
    return `${event.kind}:${event.pubkey}:${getDTagValue(event.tags)}`;
  }

  /**
   * Whether a NIP-09 deletion request already retracted this event, so it
   * must not be re-admitted.
   *
   * The id tombstone only bites when the arriving event's OWN pubkey matches
   * the pubkey that asked for the deletion; otherwise anyone could pre-block
   * an id they merely predicted. Address tombstones already carry the
   * author's pubkey inside the coordinate.
   */
  private isRetracted(event: NostrEvent): boolean {
    const tombstone = this.getTombstoneStmt.get(event.id) as
      | { pubkey: string }
      | undefined;
    if (tombstone && tombstone.pubkey === event.pubkey) return true;

    const address = this.getAddressTombstoneStmt.get(
      SqliteEventStore.coordinateOf(event)
    ) as { deleted_at: number } | undefined;
    return address !== undefined && event.created_at <= address.deleted_at;
  }

  /**
   * Apply a NIP-09 deletion request: remove the author's own targeted events
   * and record tombstones so a re-publish cannot resurrect them.
   *
   * Every statement here is scoped by `pubkey = <the requester>`, which is
   * what makes a cross-author deletion a no-op rather than a privilege.
   */
  private applyDeletion(deletion: NostrEvent): void {
    const targets = parseDeletionTargets(deletion);
    if (targets.ids.length === 0 && targets.addresses.length === 0) return;

    const deleteById = this.db.prepare(
      'DELETE FROM events WHERE id = ? AND pubkey = ? AND created_at <= ?'
    );

    const apply = this.db.transaction(() => {
      for (const id of targets.ids) {
        // Tombstone unconditionally (the event may not have arrived yet) —
        // isRetracted() enforces the same-author rule on arrival.
        this.tombstoneIdStmt.run(id, deletion.pubkey, deletion.created_at);
        deleteById.run(id, deletion.pubkey, deletion.created_at);
      }

      for (const address of targets.addresses) {
        // A coordinate naming somebody else's pubkey is ignored outright.
        if (address.pubkey !== deletion.pubkey) continue;
        const coordinate = `${address.kind}:${address.pubkey}:${address.identifier}`;
        this.tombstoneAddressStmt.run(coordinate, deletion.created_at);
        for (const row of this.findByCoordinate(address.kind, address.pubkey)) {
          if (
            getDTagValue(row.tags) === address.identifier &&
            isDeletableBy(row, deletion)
          ) {
            this.db.prepare('DELETE FROM events WHERE id = ?').run(row.id);
          }
        }
      }
    });
    apply();
  }

  /**
   * Rows for a (kind, pubkey) pair with their parsed tags, so the caller can
   * compare `d` values in code. SQL cannot distinguish `["d",""]` from a
   * missing `d` tag, and both mean "the empty identifier".
   */
  private findByCoordinate(
    kind: number,
    pubkey: string
  ): { id: string; pubkey: string; created_at: number; tags: string[][] }[] {
    const rows = this.db
      .prepare(
        'SELECT id, pubkey, created_at, tags FROM events WHERE pubkey = ? AND kind = ?'
      )
      .all(pubkey, kind) as {
      id: string;
      pubkey: string;
      created_at: number;
      tags: string;
    }[];
    return rows.map((row) => ({
      id: row.id,
      pubkey: row.pubkey,
      created_at: row.created_at,
      tags: JSON.parse(row.tags) as string[][],
    }));
  }

  /**
   * NIP-40 reaper: permanently delete events whose expiration is further than
   * `graceSeconds` in the past.
   *
   * The grace window is the safety net for enforcement itself. Serve-time
   * filtering is instantly reversible (flip `enforceExpiration` off and every
   * still-present event is served again); a DELETE is not. Keeping recently
   * expired events on disk for a while means an operator who discovers that
   * enforcement broke discovery can undo it without having lost the data.
   *
   * @param nowSeconds - Current unix time in seconds.
   * @param graceSeconds - Extra time to keep an expired event on disk.
   * @returns The number of rows deleted.
   */
  reapExpired(nowSeconds: number, graceSeconds = 0): number {
    try {
      const result = this.deleteExpiredStmt.run(nowSeconds - graceSeconds);
      return result.changes;
    } catch (error) {
      throw new RelayError(
        `Failed to reap expired events: ${error instanceof Error ? error.message : String(error)}`,
        'STORAGE_ERROR'
      );
    }
  }

  /**
   * Retrieve an event by its ID.
   *
   * Returns undefined for an event that is past its NIP-40 expiration while
   * enforcement is on, even though the row may still be on disk inside the
   * reaper's grace window.
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
            expires_at: number | null;
          }
        | undefined;

      if (!row) {
        return undefined;
      }

      if (
        this.enforceExpiration &&
        row.expires_at !== null &&
        row.expires_at <= Math.floor(Date.now() / 1000)
      ) {
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
    // NIP-40 enforcement is a WHERE clause, not a post-filter, so that a
    // filter's `limit` is applied to the events actually served. Post-filtering
    // would quietly return fewer than `limit` results and, worse, let a page
    // of expired announces displace live ones.
    const params: unknown[] = [];
    let liveClause = '';
    if (this.enforceExpiration) {
      liveClause = '(expires_at IS NULL OR expires_at > ?)';
      params.push(Math.floor(Date.now() / 1000));
    }

    if (filters.length === 0) {
      return {
        sql:
          `SELECT * FROM events${liveClause ? ` WHERE ${liveClause}` : ''}` +
          ' ORDER BY created_at DESC',
        params,
      };
    }

    const conditions: string[] = [];

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

    const whereParts: string[] = [];
    if (liveClause) whereParts.push(liveClause);
    if (conditions.length > 0) whereParts.push(`(${conditions.join(' OR ')})`);

    let sql = 'SELECT * FROM events';
    if (whereParts.length > 0) {
      sql += ` WHERE ${whereParts.join(' AND ')}`;
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
