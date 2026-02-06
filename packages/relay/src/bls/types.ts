import { RelayError } from '../storage/index.js';
import type { PricingService } from '../pricing/index.js';

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
}

/**
 * Incoming payment request from ILP connector.
 */
export interface HandlePaymentRequest {
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
 * Response for accepted payment.
 */
export interface HandlePaymentAcceptResponse {
  accept: true;
  /** Base64-encoded fulfillment (SHA-256 of event.id) */
  fulfillment: string;
  metadata?: {
    eventId: string;
    storedAt: number;
  };
}

/**
 * Response for rejected payment.
 */
export interface HandlePaymentRejectResponse {
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
 * Union type for payment response.
 */
export type HandlePaymentResponse =
  | HandlePaymentAcceptResponse
  | HandlePaymentRejectResponse;

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
export class BlsError extends RelayError {
  constructor(message: string, code = 'BLS_ERROR') {
    super(message, code);
    this.name = 'BlsError';
  }
}
