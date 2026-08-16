/**
 * NIP implementations that are policy rather than plumbing: what a relay may
 * still serve (NIP-40 expiration), what an author may retract (NIP-09
 * deletion), and the operator's narrow escape hatch for events neither NIP
 * can reach (the blocklist).
 */
export { EXPIRATION_TAG, getExpiration, isExpired } from './expiration.js';
export {
  DELETION_KIND,
  isDeletionKind,
  isDeletableBy,
  parseAddressCoordinate,
  parseDeletionTargets,
} from './deletion.js';
export type { AddressCoordinate, DeletionTargets } from './deletion.js';
export { parseBlockedEventIds } from './blocklist.js';
