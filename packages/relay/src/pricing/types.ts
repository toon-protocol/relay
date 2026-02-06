import { RelayError } from '../storage/index.js';

/**
 * Configuration for the Pricing Service.
 */
export interface PricingConfig {
  /** Base price per byte for event storage */
  basePricePerByte: bigint;
  /** Optional price overrides by event kind */
  kindOverrides?: Map<number, bigint>;
}

/**
 * Error class for pricing-specific errors.
 */
export class PricingError extends RelayError {
  constructor(message: string, code = 'PRICING_ERROR') {
    super(message, code);
    this.name = 'PricingError';
  }
}
