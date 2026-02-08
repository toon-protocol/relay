import { existsSync, accessSync, constants } from 'node:fs';
import { InMemoryEventStore } from './InMemoryEventStore.js';
import { SqliteEventStore } from './SqliteEventStore.js';

/**
 * Result of creating an event store.
 */
export interface CreateEventStoreResult {
  eventStore: SqliteEventStore | InMemoryEventStore;
  storageSummary: string;
}

/**
 * Check if a directory exists and is writable.
 */
function isDirectoryWritable(dirPath: string): boolean {
  try {
    accessSync(dirPath, constants.W_OK);
    return true;
  } catch {
    return false;
  }
}

/**
 * Create an event store with automatic fallback.
 *
 * If the data directory exists and is writable, creates a SqliteEventStore
 * with a database file at `${dataDir}/events.db`.
 *
 * If the directory does not exist or is not writable, falls back to an
 * InMemoryEventStore and returns a warning summary.
 *
 * @param dataDir - Directory path for persistent storage
 * @returns The event store instance and a human-readable summary string
 */
export function createEventStore(dataDir: string): CreateEventStoreResult {
  const dbPath = `${dataDir}/events.db`;

  if (!existsSync(dataDir)) {
    console.warn(
      `WARNING: Data directory '${dataDir}' does not exist. Using in-memory storage (events will not persist across restarts).`
    );
    return {
      eventStore: new InMemoryEventStore(),
      storageSummary: `in-memory (volatile) — directory '${dataDir}' not found`,
    };
  }

  if (!isDirectoryWritable(dataDir)) {
    console.warn(
      `WARNING: Data directory '${dataDir}' is not writable. Using in-memory storage (events will not persist across restarts).`
    );
    return {
      eventStore: new InMemoryEventStore(),
      storageSummary: `in-memory (volatile) — directory '${dataDir}' not writable`,
    };
  }

  const eventStore = new SqliteEventStore(dbPath);
  return {
    eventStore,
    storageSummary: `SQLite (${dbPath})`,
  };
}
