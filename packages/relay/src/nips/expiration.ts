/**
 * NIP-40 — "Expiration Timestamp".
 *
 * An event MAY carry `["expiration", "<unix seconds>"]`. Past that timestamp
 * the relay SHOULD stop serving it. Until relay#137 this relay parsed no such
 * tag and enforced nothing, so an announce that said "I am valid for ten
 * minutes" was still handed to every discovering client a week later (two
 * such kind:10032 announces — one of them a `g.toon.relay` from an identity
 * that no longer exists — were live on devnet when this was written).
 *
 * This module is deliberately pure: parsing and the expiry predicate only.
 * WHERE enforcement happens (serve-time filtering, the background reaper) is
 * the storage layer's and launcher's business.
 *
 * @module
 */

import type { NostrEvent } from 'nostr-tools/pure';

/** The NIP-40 tag name. */
export const EXPIRATION_TAG = 'expiration';

/**
 * Read an event's NIP-40 expiration timestamp, in unix seconds.
 *
 * FAIL-OPEN on anything malformed. A tag whose value is not a non-negative
 * integer (empty, `"soon"`, `"1.5"`, `"-1"`, absent) yields `undefined`, i.e.
 * "never expires" — the same treatment an event with no tag at all gets.
 * Dropping an event because its own author wrote a bad timestamp would turn a
 * publisher-side typo into an unrecoverable read outage, and NIP-40 asks
 * relays to honour a valid expiration, not to police an invalid one.
 *
 * The FIRST syntactically valid expiration tag wins when several are present
 * (NIP-40 does not define multi-tag behaviour; the event is signed by the
 * author about the author's own event, so there is no adversary to defend
 * against here — only a need to be deterministic).
 *
 * @param event - Any Nostr event (only `tags` is read).
 * @returns Unix-seconds expiry, or undefined when the event never expires.
 */
export function getExpiration(event: { tags: string[][] }): number | undefined {
  for (const tag of event.tags) {
    if (tag[0] !== EXPIRATION_TAG) continue;
    const raw = tag[1];
    if (raw === undefined) continue;
    // Reject anything that is not a plain non-negative integer literal.
    // `Number()` alone would happily accept '1e9', ' 12 ', '0x10' and '1.0'.
    if (!/^\d+$/.test(raw)) continue;
    const seconds = Number(raw);
    if (!Number.isSafeInteger(seconds)) continue;
    return seconds;
  }
  return undefined;
}

/**
 * Whether an event is past its NIP-40 expiration at `nowSeconds`.
 *
 * Events with no (or a malformed) expiration are never expired.
 *
 * @param event - The event to test.
 * @param nowSeconds - Current unix time in seconds.
 * @returns True when the event must no longer be served.
 */
export function isExpired(
  event: Pick<NostrEvent, 'tags'>,
  nowSeconds: number
): boolean {
  const expiration = getExpiration(event);
  return expiration !== undefined && expiration <= nowSeconds;
}
