/**
 * NIP-09 — "Event Deletion Request".
 *
 * A kind:5 event asks the relay to stop serving events the SAME pubkey
 * published, named either by id (`e` tags) or by addressable coordinate
 * (`a` tags, `<kind>:<pubkey>:<d-identifier>`).
 *
 * THE AUTHORIZATION RULE IS THE WHOLE NIP: a deletion request may only ever
 * retract events signed by its own author. A kind:5 that names someone else's
 * event id is not an error and not a partial success — the named target is
 * simply not deleted. Anything looser turns a public relay into a surface
 * where one key can erase another's history.
 *
 * This module is pure: it parses targets and answers "may this deletion
 * request retract this event?". Applying that answer to stored rows (and
 * remembering it, so a re-publish cannot resurrect the event) belongs to the
 * storage layer.
 *
 * @module
 */

import type { NostrEvent } from 'nostr-tools/pure';

/** The NIP-09 deletion-request kind. */
export const DELETION_KIND = 5;

/** Whether `kind` is a NIP-09 deletion request. */
export function isDeletionKind(kind: number): boolean {
  return kind === DELETION_KIND;
}

/**
 * A NIP-01 addressable coordinate, `<kind>:<pubkey>:<d-identifier>`, as
 * carried by a NIP-09 `a` tag.
 */
export interface AddressCoordinate {
  /** Event kind the coordinate addresses. */
  kind: number;
  /** Author pubkey (64-char lowercase hex). */
  pubkey: string;
  /** The `d` tag value; the empty string when the kind carries no `d`. */
  identifier: string;
}

/** Targets named by a deletion request. */
export interface DeletionTargets {
  /** Event ids from `e` tags (64-char lowercase hex, de-duplicated). */
  ids: string[];
  /** Coordinates from `a` tags (de-duplicated by their raw tag value). */
  addresses: AddressCoordinate[];
}

/** 64-char lowercase hex — the canonical wire form of an id or pubkey. */
const HEX_64 = /^[0-9a-f]{64}$/;

/**
 * Parse an `a`-tag value into a coordinate.
 *
 * @param value - Raw tag value, `<kind>:<pubkey>:<d-identifier>`.
 * @returns The coordinate, or undefined when the value is malformed.
 */
export function parseAddressCoordinate(
  value: string
): AddressCoordinate | undefined {
  // The identifier itself may contain ':' — split off only the first two
  // fields and keep the remainder verbatim.
  const firstSep = value.indexOf(':');
  if (firstSep < 0) return undefined;
  const secondSep = value.indexOf(':', firstSep + 1);
  if (secondSep < 0) return undefined;

  const kindPart = value.slice(0, firstSep);
  const pubkey = value.slice(firstSep + 1, secondSep);
  const identifier = value.slice(secondSep + 1);

  if (!/^\d+$/.test(kindPart)) return undefined;
  const kind = Number(kindPart);
  if (!Number.isSafeInteger(kind)) return undefined;
  if (!HEX_64.test(pubkey)) return undefined;

  return { kind, pubkey, identifier };
}

/**
 * Collect the targets a deletion request names.
 *
 * Malformed tags are skipped rather than failing the whole request: a client
 * that emits one bad `a` tag alongside three good ones still gets the three.
 *
 * @param event - A kind:5 event (the kind is not re-checked here).
 * @returns De-duplicated ids and coordinates.
 */
export function parseDeletionTargets(
  event: Pick<NostrEvent, 'tags'>
): DeletionTargets {
  const ids = new Set<string>();
  const addresses = new Map<string, AddressCoordinate>();

  for (const tag of event.tags) {
    const value = tag[1];
    if (value === undefined) continue;

    if (tag[0] === 'e') {
      if (HEX_64.test(value)) ids.add(value);
    } else if (tag[0] === 'a') {
      const coordinate = parseAddressCoordinate(value);
      if (coordinate) addresses.set(value, coordinate);
    }
  }

  return { ids: [...ids], addresses: [...addresses.values()] };
}

/**
 * Whether `deletion` is allowed to retract `target`.
 *
 * Two conditions, both required:
 *
 *  1. Same author. This is the trust boundary — see the module comment.
 *  2. `target.created_at <= deletion.created_at`. A deletion request cannot
 *     pre-emptively retract a future event; without this, one kind:5 would
 *     permanently silence every later event a key publishes.
 *
 * @param target - The stored event being considered for retraction.
 * @param deletion - The kind:5 deletion request.
 * @returns True when the retraction is authorized.
 */
export function isDeletableBy(
  target: Pick<NostrEvent, 'pubkey' | 'created_at'>,
  deletion: Pick<NostrEvent, 'pubkey' | 'created_at'>
): boolean {
  return (
    target.pubkey === deletion.pubkey &&
    target.created_at <= deletion.created_at
  );
}
