import type { NostrEvent } from 'nostr-tools/pure';
import type { Filter } from 'nostr-tools/filter';
import { matchFilter } from '../filters/index.js';
import type { EventStore } from './EventStore.js';

/**
 * In-memory implementation of EventStore.
 * Events are stored in a Map keyed by event ID.
 */
export class InMemoryEventStore implements EventStore {
  private events = new Map<string, NostrEvent>();

  store(event: NostrEvent): void {
    this.events.set(event.id, event);
  }

  get(id: string): NostrEvent | undefined {
    return this.events.get(id);
  }

  query(filters: Filter[]): NostrEvent[] {
    // Get all events
    const allEvents = Array.from(this.events.values());

    // If no filters provided, return all events sorted by created_at desc
    if (filters.length === 0) {
      return allEvents.sort((a, b) => b.created_at - a.created_at);
    }

    // Find events matching ANY filter (OR logic between filters)
    const matchingEvents: NostrEvent[] = [];

    for (const event of allEvents) {
      for (const filter of filters) {
        if (matchFilter(event, filter)) {
          matchingEvents.push(event);
          break; // Only add once even if matches multiple filters
        }
      }
    }

    // Sort by created_at descending
    matchingEvents.sort((a, b) => b.created_at - a.created_at);

    // Apply limit from first filter that has one (NIP-01 semantics)
    const limitFilter = filters.find((f) => f.limit !== undefined);
    if (limitFilter?.limit !== undefined) {
      return matchingEvents.slice(0, limitFilter.limit);
    }

    return matchingEvents;
  }

  /**
   * Close the storage backend (no-op for in-memory store).
   */
  close(): void {
    // No-op for in-memory store
  }
}
