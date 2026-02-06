import { describe, it, expect, beforeEach } from 'vitest';
import { generateSecretKey, finalizeEvent } from 'nostr-tools/pure';
import type { NostrEvent } from 'nostr-tools/pure';
import { PricingService } from './PricingService.js';
import { PricingError } from './types.js';
import { encodeEventToToon } from '../toon/index.js';

/**
 * Helper to create a valid signed test event.
 */
function createTestEvent(kind: number, content: string): NostrEvent {
  const sk = generateSecretKey();
  return finalizeEvent(
    {
      kind,
      content,
      tags: [],
      created_at: Math.floor(Date.now() / 1000),
    },
    sk
  );
}

describe('PricingService', () => {
  describe('constructor validation', () => {
    it('should throw PricingError for negative basePricePerByte', () => {
      expect(
        () => new PricingService({ basePricePerByte: -1n })
      ).toThrowError(PricingError);
      expect(
        () => new PricingService({ basePricePerByte: -1n })
      ).toThrowError('basePricePerByte must be non-negative');
    });

    it('should throw PricingError for negative kind override values', () => {
      expect(
        () =>
          new PricingService({
            basePricePerByte: 10n,
            kindOverrides: new Map([[1, -5n]]),
          })
      ).toThrowError(PricingError);
      expect(
        () =>
          new PricingService({
            basePricePerByte: 10n,
            kindOverrides: new Map([[1, -5n]]),
          })
      ).toThrowError('kindOverride for kind 1 must be non-negative');
    });

    it('should accept valid config with base price only', () => {
      const service = new PricingService({ basePricePerByte: 10n });
      expect(service).toBeInstanceOf(PricingService);
    });

    it('should accept valid config with kind overrides', () => {
      const service = new PricingService({
        basePricePerByte: 10n,
        kindOverrides: new Map([
          [0, 0n],
          [30023, 100n],
        ]),
      });
      expect(service).toBeInstanceOf(PricingService);
    });

    it('should accept zero as basePricePerByte', () => {
      const service = new PricingService({ basePricePerByte: 0n });
      expect(service).toBeInstanceOf(PricingService);
    });
  });

  describe('with base price only', () => {
    let service: PricingService;

    beforeEach(() => {
      service = new PricingService({ basePricePerByte: 10n });
    });

    it('should calculate price based on TOON byte size', () => {
      const event = createTestEvent(1, 'hello');
      const price = service.calculatePrice(event);
      expect(price).toBeGreaterThan(0n);
    });

    it('should return consistent prices for same event', () => {
      const event = createTestEvent(1, 'test content');
      const price1 = service.calculatePrice(event);
      const price2 = service.calculatePrice(event);
      expect(price1).toBe(price2);
    });

    it('should calculate correct price from bytes and kind', () => {
      const event = createTestEvent(1, 'test');
      const toonBytes = encodeEventToToon(event);
      const expectedPrice = BigInt(toonBytes.length) * 10n;

      const price = service.calculatePriceFromBytes(toonBytes, event.kind);
      expect(price).toBe(expectedPrice);
    });

    it('should return base price per byte for any kind', () => {
      expect(service.getPricePerByte(1)).toBe(10n);
      expect(service.getPricePerByte(0)).toBe(10n);
      expect(service.getPricePerByte(30023)).toBe(10n);
      expect(service.getPricePerByte(10032)).toBe(10n);
    });
  });

  describe('with kind overrides', () => {
    let service: PricingService;

    beforeEach(() => {
      service = new PricingService({
        basePricePerByte: 10n,
        kindOverrides: new Map([
          [0, 0n], // Free profiles
          [30023, 100n], // Expensive articles
          [1, 5n], // Cheaper notes
        ]),
      });
    });

    it('should use kind override for kind 0 (free)', () => {
      const profile = createTestEvent(0, '{"name":"test"}');
      expect(service.calculatePrice(profile)).toBe(0n);
    });

    it('should use kind override for kind 30023 (expensive)', () => {
      const article = createTestEvent(30023, 'Long article content');
      const toonBytes = encodeEventToToon(article);
      const expectedPrice = BigInt(toonBytes.length) * 100n;
      expect(service.calculatePrice(article)).toBe(expectedPrice);
    });

    it('should use kind override for kind 1 (cheaper)', () => {
      const note = createTestEvent(1, 'hello');
      const toonBytes = encodeEventToToon(note);
      const expectedPrice = BigInt(toonBytes.length) * 5n;
      expect(service.calculatePrice(note)).toBe(expectedPrice);
    });

    it('should fall back to base price for unlisted kinds', () => {
      const event = createTestEvent(7, 'reaction');
      const toonBytes = encodeEventToToon(event);
      const expectedPrice = BigInt(toonBytes.length) * 10n;
      expect(service.calculatePrice(event)).toBe(expectedPrice);
    });

    it('should return correct price per byte for each kind', () => {
      expect(service.getPricePerByte(0)).toBe(0n);
      expect(service.getPricePerByte(1)).toBe(5n);
      expect(service.getPricePerByte(30023)).toBe(100n);
      expect(service.getPricePerByte(7)).toBe(10n); // Falls back to base
    });
  });

  describe('with various event kinds', () => {
    let service: PricingService;

    beforeEach(() => {
      service = new PricingService({
        basePricePerByte: 10n,
        kindOverrides: new Map([
          [10032, 20n], // ILP Peer Info
          [10047, 15n], // SPSP Info
        ]),
      });
    });

    it('should handle kind 10032 (ILP Peer Info)', () => {
      const event = createTestEvent(10032, '{"ilp_address":"g.test"}');
      const toonBytes = encodeEventToToon(event);
      const expectedPrice = BigInt(toonBytes.length) * 20n;
      expect(service.calculatePrice(event)).toBe(expectedPrice);
    });

    it('should handle kind 10047 (SPSP Info)', () => {
      const event = createTestEvent(10047, '{"destination_account":"g.test"}');
      const toonBytes = encodeEventToToon(event);
      const expectedPrice = BigInt(toonBytes.length) * 15n;
      expect(service.calculatePrice(event)).toBe(expectedPrice);
    });

    it('should handle kind 23194 (SPSP Request) with base price', () => {
      const event = createTestEvent(23194, '{}');
      const toonBytes = encodeEventToToon(event);
      const expectedPrice = BigInt(toonBytes.length) * 10n;
      expect(service.calculatePrice(event)).toBe(expectedPrice);
    });
  });

  describe('with large events', () => {
    let service: PricingService;

    beforeEach(() => {
      service = new PricingService({ basePricePerByte: 1000000n }); // 1 million per byte
    });

    it('should handle large events correctly with bigint', () => {
      // Create an event with substantial content
      const largeContent = 'x'.repeat(10000); // 10KB content
      const event = createTestEvent(1, largeContent);
      const toonBytes = encodeEventToToon(event);
      const expectedPrice = BigInt(toonBytes.length) * 1000000n;

      const price = service.calculatePrice(event);
      expect(price).toBe(expectedPrice);
      expect(price).toBeGreaterThan(10000000000n); // > 10 billion
    });

    it('should not overflow with large prices', () => {
      const content = 'y'.repeat(1000);
      const event = createTestEvent(1, content);
      const price = service.calculatePrice(event);

      // Verify it's a valid bigint and not undefined/NaN
      expect(typeof price).toBe('bigint');
      expect(price).toBeGreaterThan(0n);
    });
  });

  describe('with zero byte price (free events)', () => {
    let service: PricingService;

    beforeEach(() => {
      service = new PricingService({ basePricePerByte: 0n });
    });

    it('should calculate zero price for any event', () => {
      const event = createTestEvent(1, 'this should be free');
      expect(service.calculatePrice(event)).toBe(0n);
    });

    it('should return zero price per byte', () => {
      expect(service.getPricePerByte(1)).toBe(0n);
      expect(service.getPricePerByte(30023)).toBe(0n);
    });
  });

  describe('calculatePriceFromBytes', () => {
    it('should calculate correct price from raw bytes', () => {
      const service = new PricingService({ basePricePerByte: 5n });
      const bytes = new Uint8Array(100);
      expect(service.calculatePriceFromBytes(bytes, 1)).toBe(500n);
    });

    it('should use kind override when calculating from bytes', () => {
      const service = new PricingService({
        basePricePerByte: 5n,
        kindOverrides: new Map([[1, 20n]]),
      });
      const bytes = new Uint8Array(100);
      expect(service.calculatePriceFromBytes(bytes, 1)).toBe(2000n);
    });

    it('should fall back to base price when kind not in overrides', () => {
      const service = new PricingService({
        basePricePerByte: 5n,
        kindOverrides: new Map([[1, 20n]]),
      });
      const bytes = new Uint8Array(100);
      expect(service.calculatePriceFromBytes(bytes, 7)).toBe(500n);
    });

    it('should handle empty bytes array', () => {
      const service = new PricingService({ basePricePerByte: 10n });
      const bytes = new Uint8Array(0);
      expect(service.calculatePriceFromBytes(bytes, 1)).toBe(0n);
    });
  });
});
