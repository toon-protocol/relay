import { describe, it, expect, afterEach } from 'vitest';
import { mkdtempSync, rmSync, chmodSync } from 'node:fs';
import { join } from 'node:path';
import { tmpdir } from 'node:os';
import { createEventStore } from './createEventStore.js';
import { SqliteEventStore } from './SqliteEventStore.js';
import { InMemoryEventStore } from './InMemoryEventStore.js';

describe('createEventStore', () => {
  let tempDir: string | undefined;
  let closeStore: (() => void) | undefined;

  afterEach(() => {
    if (closeStore) {
      closeStore();
      closeStore = undefined;
    }
    if (tempDir) {
      try {
        // Restore write permission before cleanup (needed for read-only test)
        chmodSync(tempDir, 0o755);
      } catch {
        // Ignore if already gone
      }
      rmSync(tempDir, { recursive: true, force: true });
      tempDir = undefined;
    }
  });

  it('should return SqliteEventStore when dataDir is writable', () => {
    tempDir = mkdtempSync(join(tmpdir(), 'bls-test-'));
    const { eventStore, storageSummary } = createEventStore(tempDir);
    closeStore = () => eventStore.close();

    expect(eventStore).toBeInstanceOf(SqliteEventStore);
    expect(storageSummary).toContain('SQLite');
    expect(storageSummary).toContain(`${tempDir}/events.db`);
  });

  it('should return InMemoryEventStore when dataDir does not exist', () => {
    const nonExistentDir = join(tmpdir(), 'bls-test-nonexistent-' + Date.now());
    const { eventStore, storageSummary } = createEventStore(nonExistentDir);
    closeStore = () => eventStore.close();

    expect(eventStore).toBeInstanceOf(InMemoryEventStore);
    expect(storageSummary).toContain('in-memory');
    expect(storageSummary).toContain('volatile');
    expect(storageSummary).toContain('not found');
  });

  it('should return InMemoryEventStore when dataDir is not writable', () => {
    tempDir = mkdtempSync(join(tmpdir(), 'bls-test-'));
    chmodSync(tempDir, 0o444); // Read-only

    const { eventStore, storageSummary } = createEventStore(tempDir);
    closeStore = () => eventStore.close();

    expect(eventStore).toBeInstanceOf(InMemoryEventStore);
    expect(storageSummary).toContain('in-memory');
    expect(storageSummary).toContain('volatile');
    expect(storageSummary).toContain('not writable');
  });

  it('should include storage type in summary for SQLite', () => {
    tempDir = mkdtempSync(join(tmpdir(), 'bls-test-'));
    const { storageSummary } = createEventStore(tempDir);
    closeStore = () => {}; // SQLite store will be GC'd

    expect(storageSummary).toMatch(/^SQLite \(/);
  });

  it('should include storage type in summary for in-memory fallback', () => {
    const nonExistentDir = join(tmpdir(), 'bls-test-nonexistent-' + Date.now());
    const { storageSummary } = createEventStore(nonExistentDir);

    expect(storageSummary).toMatch(/^in-memory \(/);
  });
});
