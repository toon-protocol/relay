import type { NostrEvent } from 'nostr-tools/pure';
import type { Filter } from 'nostr-tools/filter';

/**
 * Interface for event storage backends.
 */
export interface EventStore {
  /** Store an event by its ID */
  store(event: NostrEvent): void;
  /** Retrieve a single event by ID */
  get(id: string): NostrEvent | undefined;
  /** Query events matching any of the provided filters */
  query(filters: Filter[]): NostrEvent[];
  /** Close the storage backend (optional) */
  close?(): void;
}
