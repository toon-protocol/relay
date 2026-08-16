import type { NostrEvent } from 'nostr-tools/pure';
import type { Filter } from 'nostr-tools/filter';
import { matchFilter } from '../filters/index.js';
import { isExpired } from '../nips/expiration.js';
import {
  isDeletionKind,
  isDeletableBy,
  parseDeletionTargets,
} from '../nips/deletion.js';

/**
 * Construction options shared by every EventStore implementation.
 */
export interface EventStoreOptions {
  /**
   * Enforce NIP-40 expiration at serve time (default: true).
   *
   * When true, events past their `expiration` tag are not returned by
   * `get()` or `query()`. Set false as a KILL SWITCH: it restores the
   * pre-NIP-40 behaviour of serving every stored event forever, which is the
   * only recourse if enforcement ever starves discovery (see the launcher's
   * `enforceExpiration` docs for the failure mode this guards against).
   */
  enforceExpiration?: boolean;
  /**
   * Operator-blocked event ids (64-char lowercase hex). Blocked events are
   * refused on write and swept from storage. See `nips/blocklist.ts` for why
   * this is scoped to ids and to startup configuration.
   */
  blockedEventIds?: Iterable<string>;
}

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
  /**
   * Permanently drop events expired for longer than `graceSeconds` (NIP-40).
   * Optional: a backend may serve-filter only. Returns rows removed.
   */
  reapExpired?(nowSeconds: number, graceSeconds?: number): number;
  /** Close the storage backend (optional) */
  close?(): void;
}

/**
 * In-memory implementation of EventStore.
 * Events are stored in a Map keyed by event ID.
 */
export class InMemoryEventStore implements EventStore {
  private events = new Map<string, NostrEvent>();
  /** NIP-09 id tombstones: event id -> the pubkey that requested deletion. */
  private deletedIds = new Map<string, string>();
  /** NIP-09 address tombstones: `<kind>:<pubkey>:<d>` -> deletion created_at. */
  private deletedAddresses = new Map<string, number>();
  private readonly enforceExpiration: boolean;
  private readonly blockedEventIds: ReadonlySet<string>;

  constructor(options: EventStoreOptions = {}) {
    this.enforceExpiration = options.enforceExpiration ?? true;
    this.blockedEventIds = new Set(options.blockedEventIds ?? []);
  }

  store(event: NostrEvent): void {
    if (this.blockedEventIds.has(event.id)) return;
    if (this.isRetracted(event)) return;

    if (isDeletionKind(event.kind)) {
      this.applyDeletion(event);
    }

    this.events.set(event.id, event);
  }

  get(id: string): NostrEvent | undefined {
    const event = this.events.get(id);
    if (!event) return undefined;
    if (this.enforceExpiration && isExpired(event, nowSeconds())) {
      return undefined;
    }
    return event;
  }

  query(filters: Filter[]): NostrEvent[] {
    const now = nowSeconds();
    // Get all events, minus anything NIP-40 says we may no longer serve.
    const allEvents = Array.from(this.events.values()).filter(
      (event) => !this.enforceExpiration || !isExpired(event, now)
    );

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
   * Drop events expired for longer than `graceSeconds` (NIP-40).
   *
   * @param now - Current unix time in seconds.
   * @param graceSeconds - Extra time an expired event is kept.
   * @returns The number of events removed.
   */
  reapExpired(now: number, graceSeconds = 0): number {
    let removed = 0;
    for (const [id, event] of this.events) {
      if (isExpired(event, now - graceSeconds)) {
        this.events.delete(id);
        removed++;
      }
    }
    return removed;
  }

  /** The NIP-01 addressable coordinate of an event, `<kind>:<pubkey>:<d>`. */
  private static coordinateOf(event: NostrEvent): string {
    const identifier = event.tags.find((tag) => tag[0] === 'd')?.[1] ?? '';
    return `${event.kind}:${event.pubkey}:${identifier}`;
  }

  /** Whether a NIP-09 request already retracted this event (same author). */
  private isRetracted(event: NostrEvent): boolean {
    if (this.deletedIds.get(event.id) === event.pubkey) return true;
    const deletedAt = this.deletedAddresses.get(
      InMemoryEventStore.coordinateOf(event)
    );
    return deletedAt !== undefined && event.created_at <= deletedAt;
  }

  /** Apply a kind:5 request to the author's OWN events only. */
  private applyDeletion(deletion: NostrEvent): void {
    const targets = parseDeletionTargets(deletion);

    for (const id of targets.ids) {
      this.deletedIds.set(id, deletion.pubkey);
      const target = this.events.get(id);
      if (target && isDeletableBy(target, deletion)) {
        this.events.delete(id);
      }
    }

    for (const address of targets.addresses) {
      if (address.pubkey !== deletion.pubkey) continue;
      const coordinate = `${address.kind}:${address.pubkey}:${address.identifier}`;
      this.deletedAddresses.set(
        coordinate,
        Math.max(
          this.deletedAddresses.get(coordinate) ?? deletion.created_at,
          deletion.created_at
        )
      );
      for (const [id, target] of this.events) {
        if (
          InMemoryEventStore.coordinateOf(target) === coordinate &&
          isDeletableBy(target, deletion)
        ) {
          this.events.delete(id);
        }
      }
    }
  }

  /**
   * Close the storage backend (no-op for in-memory store).
   */
  close(): void {
    // No-op for in-memory store
  }
}

/** Current unix time in seconds. */
function nowSeconds(): number {
  return Math.floor(Date.now() / 1000);
}
