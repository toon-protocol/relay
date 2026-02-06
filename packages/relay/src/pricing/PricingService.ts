import type { NostrEvent } from 'nostr-tools/pure';
import { encodeEventToToon } from '../toon/index.js';
import type { PricingConfig } from './types.js';
import { PricingError } from './types.js';

/**
 * Service for calculating event storage prices with kind-based overrides.
 */
export class PricingService {
  private readonly basePricePerByte: bigint;
  private readonly kindOverrides: Map<number, bigint>;

  constructor(config: PricingConfig) {
    // Validate basePricePerByte is non-negative
    if (config.basePricePerByte < 0n) {
      throw new PricingError(
        'basePricePerByte must be non-negative',
        'INVALID_CONFIG'
      );
    }

    // Validate all kindOverrides are non-negative
    if (config.kindOverrides) {
      for (const [kind, price] of config.kindOverrides.entries()) {
        if (price < 0n) {
          throw new PricingError(
            `kindOverride for kind ${kind} must be non-negative`,
            'INVALID_CONFIG'
          );
        }
      }
    }

    this.basePricePerByte = config.basePricePerByte;
    this.kindOverrides = config.kindOverrides ?? new Map();
  }

  /**
   * Calculate price for a Nostr event.
   *
   * @param event - The Nostr event to price
   * @returns The calculated price as bigint
   */
  calculatePrice(event: NostrEvent): bigint {
    const toonBytes = encodeEventToToon(event);
    return this.calculatePriceFromBytes(toonBytes, event.kind);
  }

  /**
   * Calculate price from raw TOON bytes and event kind.
   *
   * @param bytes - The TOON-encoded event bytes
   * @param kind - The event kind number
   * @returns The calculated price as bigint
   */
  calculatePriceFromBytes(bytes: Uint8Array, kind: number): bigint {
    const pricePerByte = this.getPricePerByte(kind);
    return BigInt(bytes.length) * pricePerByte;
  }

  /**
   * Get the effective price per byte for a given kind.
   *
   * @param kind - The event kind number
   * @returns The price per byte (kind override if exists, otherwise base price)
   */
  getPricePerByte(kind: number): bigint {
    return this.kindOverrides.get(kind) ?? this.basePricePerByte;
  }
}
