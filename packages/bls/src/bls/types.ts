import { BlsBaseError } from '../errors.js';
import type { PricingService } from '../pricing/index.js';

/**
 * SPSP request event kind (NIP-proposed kind:23194).
 * Defined locally since BLS does not import from @crosstown/core.
 */
export const SPSP_REQUEST_KIND = 23194;

/**
 * Regex for validating Nostr pubkeys (64 lowercase hex characters).
 */
export const PUBKEY_REGEX = /^[0-9a-f]{64}$/;

/**
 * Validate that a string is a valid Nostr pubkey format.
 * @param pubkey - The pubkey to validate
 * @returns true if valid 64-character lowercase hex string
 */
export function isValidPubkey(pubkey: string): boolean {
  return PUBKEY_REGEX.test(pubkey);
}

/**
 * Configuration for the Business Logic Server.
 */
export interface BlsConfig {
  /** Base price per byte for event storage (used for simple pricing) */
  basePricePerByte: bigint;
  /** Optional PricingService for kind-based pricing overrides */
  pricingService?: PricingService;
  /** Optional owner pubkey - events from this pubkey bypass payment */
  ownerPubkey?: string;
  /** Optional minimum price for SPSP request events (kind:23194). When set to 0n, SPSP requests are accepted without payment. Defaults to standard pricing when undefined. */
  spspMinPrice?: bigint;
}

/**
 * Incoming packet request from ILP connector.
 */
export interface HandlePacketRequest {
  /** Payment amount as string (parsed to bigint) */
  amount: string;
  /** ILP destination address */
  destination: string;
  /** Base64-encoded TOON Nostr event */
  data: string;
  /** Source ILP address */
  sourceAccount?: string;
}

/**
 * Response for accepted packet.
 */
export interface HandlePacketAcceptResponse {
  accept: true;
  /** Base64-encoded fulfillment (SHA-256 of event.id) */
  fulfillment: string;
  metadata?: {
    eventId: string;
    storedAt: number;
  };
}

/**
 * Response for rejected packet.
 */
export interface HandlePacketRejectResponse {
  accept: false;
  /** ILP error code (F00, F06, etc.) */
  code: string;
  /** Human-readable error message */
  message: string;
  metadata?: {
    required?: string;
    received?: string;
  };
}

/**
 * Union type for packet response.
 */
export type HandlePacketResponse =
  | HandlePacketAcceptResponse
  | HandlePacketRejectResponse;

/**
 * ILP error code constants.
 */
export const ILP_ERROR_CODES = {
  BAD_REQUEST: 'F00',
  INSUFFICIENT_AMOUNT: 'F06',
  INTERNAL_ERROR: 'T00',
} as const;

/**
 * BLS-specific error class.
 */
export class BlsError extends BlsBaseError {
  constructor(message: string, code = 'BLS_ERROR') {
    super(message, code);
    this.name = 'BlsError';
  }
}
