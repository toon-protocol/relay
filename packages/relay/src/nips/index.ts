/**
 * NIP implementations that are policy rather than plumbing: what a relay may
 * still serve (NIP-40 expiration), what an author may retract (NIP-09
 * deletion), the operator's narrow escape hatch for events neither NIP can
 * reach (the blocklist), and what the relay says about itself and about where
 * a write to it is paid for (NIP-11).
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
export {
  acceptsRelayInformation,
  buildRelayInformationDocument,
  writeRefusalMessage,
  NOSTR_JSON_CONTENT_TYPE,
  RELAY_SOFTWARE_URL,
} from './relay-information.js';
export type {
  Carriage,
  RelayDescription,
  RelayInformationDocument,
  RelayInformationInput,
  RelayLimitation,
  RelaySettlement,
  RelayWriteEdge,
} from './relay-information.js';
