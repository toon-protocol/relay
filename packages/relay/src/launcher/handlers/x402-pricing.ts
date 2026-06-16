/**
 * x402 pricing calculator with multi-hop routing buffer.
 *
 * Computes the all-in USDC price for publishing a Nostr event via the
 * x402 HTTP on-ramp. The price includes a configurable routing buffer
 * (default 10%) to cover multi-hop overhead -- intermediate relays charge
 * their own per-byte fees.
 *
 * @module
 */

/**
 * Configuration for the x402 pricing calculator.
 */
export interface X402PricingConfig {
  /** Base price per byte in ILP/USDC micro-units (e.g., 10n). */
  basePricePerByte: bigint;
  /** Routing buffer percentage (default: 10, meaning 10%). */
  routingBufferPercent: number;
}

/**
 * Calculate the all-in x402 price for a TOON payload.
 *
 * Formula:
 *   basePrice = basePricePerByte * toonLength
 *   buffer    = basePrice * routingBufferPercent / 100
 *   total     = basePrice + buffer
 *
 * The routing buffer covers multi-hop overhead. 10% default is a
 * conservative estimate per Party Mode Decision 8.
 *
 * @param config - Pricing configuration with base price and buffer percent.
 * @param toonLength - Length of the TOON-encoded payload in bytes.
 * @returns Total price in USDC micro-units.
 */
export function calculateX402Price(
  config: X402PricingConfig,
  toonLength: number
): bigint {
  // Guard against misconfigured routing buffer that could undercharge or
  // produce nonsensical prices. Clamp to [0, 200] (0% to 200% buffer).
  const clampedBuffer = Math.max(0, Math.min(200, config.routingBufferPercent));
  const basePrice = config.basePricePerByte * BigInt(toonLength);
  const buffer = (basePrice * BigInt(clampedBuffer)) / 100n;
  return basePrice + buffer;
}
