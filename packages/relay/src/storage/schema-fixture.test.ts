/**
 * Keeps the Rust store's fixture honest (#193).
 *
 * `crates/relay/tests/fixtures/typescript-schema.sql` stands in for a database
 * this relay created, so the Rust relay (#185) can be tested against the
 * schema it must open unchanged. A fixture written by hand would drift from
 * the schema it claims to be; this test builds one database from the fixture
 * and one from SqliteEventStore and requires SQLite to have recorded the same
 * statements for both.
 */

import { describe, it, expect } from 'vitest';
import Database from 'better-sqlite3';
import { mkdtempSync, readFileSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { SqliteEventStore } from './SqliteEventStore.js';

const FIXTURE = resolve(
  import.meta.dirname,
  '../../../../crates/relay/tests/fixtures/typescript-schema.sql'
);

/** Every statement SQLite recorded for the database, in creation order. */
function recordedSchema(db: Database.Database): string[] {
  const rows = db
    .prepare(
      'SELECT sql FROM sqlite_master WHERE sql IS NOT NULL ORDER BY rowid'
    )
    .all() as { sql: string }[];
  return rows.map((row) => row.sql);
}

describe('the Rust store fixture', () => {
  it('is the schema SqliteEventStore creates', () => {
    const dir = mkdtempSync(join(tmpdir(), 'schema-fixture-'));
    try {
      const path = join(dir, 'events.db');
      new SqliteEventStore(path).close();
      const created = new Database(path, { readonly: true });
      const fromFixture = new Database(':memory:');
      fromFixture.exec(readFileSync(FIXTURE, 'utf8'));

      expect(recordedSchema(fromFixture)).toEqual(recordedSchema(created));
      expect(recordedSchema(created).length).toBeGreaterThan(0);

      created.close();
      fromFixture.close();
    } finally {
      rmSync(dir, { recursive: true, force: true });
    }
  });
});
