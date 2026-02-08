import type { NostrEvent } from 'nostr-tools/pure';
import type { Filter } from 'nostr-tools/filter';

/**
 * Check if an event matches a single filter according to NIP-01 rules.
 *
 * Matching rules:
 * - All specified fields must match (AND logic)
 * - `ids` and `authors` support prefix matching
 * - Tag filters (#e, #p, etc.) match events with corresponding tags
 * - Empty filter matches all events
 *
 * @param event - The Nostr event to check
 * @param filter - The filter to match against
 * @returns true if the event matches the filter
 */
export function matchFilter(event: NostrEvent, filter: Filter): boolean {
  // Empty filter matches everything
  if (Object.keys(filter).length === 0) {
    return true;
  }

  // Check ids (prefix matching)
  if (filter.ids !== undefined && filter.ids.length > 0) {
    const matches = filter.ids.some((id) => event.id.startsWith(id));
    if (!matches) return false;
  }

  // Check authors (prefix matching)
  if (filter.authors !== undefined && filter.authors.length > 0) {
    const matches = filter.authors.some((author) =>
      event.pubkey.startsWith(author)
    );
    if (!matches) return false;
  }

  // Check kinds (exact matching)
  if (filter.kinds !== undefined && filter.kinds.length > 0) {
    if (!filter.kinds.includes(event.kind)) return false;
  }

  // Check since (created_at >= since)
  if (filter.since !== undefined) {
    if (event.created_at < filter.since) return false;
  }

  // Check until (created_at <= until)
  if (filter.until !== undefined) {
    if (event.created_at > filter.until) return false;
  }

  // Check tag filters (#e, #p, and generic #<single-letter>)
  for (const key of Object.keys(filter)) {
    if (key.startsWith('#') && key.length === 2) {
      const tagName = key.slice(1);
      const filterValues = filter[key as `#${string}`];

      if (filterValues !== undefined && filterValues.length > 0) {
        // Find matching tags in the event
        const eventTagValues = event.tags
          .filter((tag) => tag[0] === tagName)
          .map((tag) => tag[1]);

        // At least one filter value must match an event tag value
        const hasMatch = filterValues.some((v) => eventTagValues.includes(v));
        if (!hasMatch) return false;
      }
    }
  }

  return true;
}
