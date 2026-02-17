import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { loadBlsConfigFromEnv } from './config.js';
import { ConfigError } from './errors.js';
import { PricingError } from './pricing/types.js';

// Known test keypair (deterministic)
const TEST_SECRET_KEY = 'a'.repeat(64);
const TEST_PUBKEY = '6a04ab98d9e4774ad806e302dddeb63bea16b5cb5f223ee77478e861bb583eb3';

/** Set all required env vars for a valid config. */
function setRequiredEnv(): void {
  process.env['NODE_ID'] = 'test-node-1';
  process.env['NOSTR_SECRET_KEY'] = TEST_SECRET_KEY;
  process.env['ILP_ADDRESS'] = 'g.crosstown.test';
}

describe('loadBlsConfigFromEnv', () => {
  const originalEnv = process.env;

  beforeEach(() => {
    vi.resetModules();
    process.env = { ...originalEnv };
    // Clear all relevant env vars
    delete process.env['NODE_ID'];
    delete process.env['NOSTR_SECRET_KEY'];
    delete process.env['ILP_ADDRESS'];
    delete process.env['BLS_PORT'];
    delete process.env['BLS_BASE_PRICE_PER_BYTE'];
    delete process.env['RELAY_BASE_PRICE_PER_BYTE'];
    delete process.env['BASE_PRICE_PER_BYTE'];
    delete process.env['OWNER_PUBKEY'];
    delete process.env['DATA_DIR'];
    delete process.env['BLS_KIND_OVERRIDES'];
    delete process.env['RELAY_KIND_OVERRIDES'];
    delete process.env['KIND_OVERRIDES'];
    delete process.env['SPSP_MIN_PRICE'];
  });

  afterEach(() => {
    process.env = originalEnv;
  });

  // --- Success path ---

  it('should return valid config when all required vars are set', () => {
    setRequiredEnv();

    const config = loadBlsConfigFromEnv();

    expect(config.nodeId).toBe('test-node-1');
    expect(config.nostrSecretKey).toBe(TEST_SECRET_KEY);
    expect(config.pubkey).toBe(TEST_PUBKEY);
    expect(config.ilpAddress).toBe('g.crosstown.test');
    expect(config.port).toBe(3100);
    expect(config.basePricePerByte).toBe(10n);
    expect(config.ownerPubkey).toBeUndefined();
    expect(config.dataDir).toBe('/data');
    expect(config.kindOverrides).toBeUndefined();
  });

  it('should derive correct pubkey from secret key', () => {
    setRequiredEnv();

    const config = loadBlsConfigFromEnv();

    expect(config.pubkey).toBe(TEST_PUBKEY);
  });

  // --- NODE_ID ---

  it('should throw ConfigError when NODE_ID is missing', () => {
    setRequiredEnv();
    delete process.env['NODE_ID'];

    expect(() => loadBlsConfigFromEnv()).toThrow(ConfigError);
    expect(() => loadBlsConfigFromEnv()).toThrow('NODE_ID: is required but not set');
  });

  // --- NOSTR_SECRET_KEY ---

  it('should throw ConfigError when NOSTR_SECRET_KEY is missing', () => {
    setRequiredEnv();
    delete process.env['NOSTR_SECRET_KEY'];

    expect(() => loadBlsConfigFromEnv()).toThrow(ConfigError);
    expect(() => loadBlsConfigFromEnv()).toThrow('NOSTR_SECRET_KEY: is required but not set');
  });

  it('should throw ConfigError when NOSTR_SECRET_KEY is too short', () => {
    setRequiredEnv();
    process.env['NOSTR_SECRET_KEY'] = 'abcd1234';

    expect(() => loadBlsConfigFromEnv()).toThrow(ConfigError);
    expect(() => loadBlsConfigFromEnv()).toThrow('must be a 64-character hex string');
  });

  it('should throw ConfigError when NOSTR_SECRET_KEY is too long', () => {
    setRequiredEnv();
    process.env['NOSTR_SECRET_KEY'] = 'a'.repeat(128);

    expect(() => loadBlsConfigFromEnv()).toThrow(ConfigError);
    expect(() => loadBlsConfigFromEnv()).toThrow('must be a 64-character hex string');
  });

  it('should throw ConfigError when NOSTR_SECRET_KEY has non-hex characters', () => {
    setRequiredEnv();
    process.env['NOSTR_SECRET_KEY'] = 'g'.repeat(64);

    expect(() => loadBlsConfigFromEnv()).toThrow(ConfigError);
    expect(() => loadBlsConfigFromEnv()).toThrow('must be a 64-character hex string');
  });

  it('should normalize NOSTR_SECRET_KEY to lowercase', () => {
    setRequiredEnv();
    process.env['NOSTR_SECRET_KEY'] = 'A'.repeat(64);

    const config = loadBlsConfigFromEnv();

    expect(config.nostrSecretKey).toBe('a'.repeat(64));
    expect(config.pubkey).toBe(TEST_PUBKEY);
  });

  // --- ILP_ADDRESS ---

  it('should throw ConfigError when ILP_ADDRESS is missing', () => {
    setRequiredEnv();
    delete process.env['ILP_ADDRESS'];

    expect(() => loadBlsConfigFromEnv()).toThrow(ConfigError);
    expect(() => loadBlsConfigFromEnv()).toThrow('ILP_ADDRESS: is required but not set');
  });

  it('should throw ConfigError when ILP_ADDRESS has invalid format (no g. prefix)', () => {
    setRequiredEnv();
    process.env['ILP_ADDRESS'] = 'invalid-address';

    expect(() => loadBlsConfigFromEnv()).toThrow(ConfigError);
    expect(() => loadBlsConfigFromEnv()).toThrow('must start with "g."');
  });

  it('should throw ConfigError when ILP_ADDRESS has invalid characters', () => {
    setRequiredEnv();
    process.env['ILP_ADDRESS'] = 'g.invalid address!';

    expect(() => loadBlsConfigFromEnv()).toThrow(ConfigError);
    expect(() => loadBlsConfigFromEnv()).toThrow('must start with "g."');
  });

  // --- BLS_PORT ---

  it('should use default port 3100 when BLS_PORT is not set', () => {
    setRequiredEnv();

    const config = loadBlsConfigFromEnv();

    expect(config.port).toBe(3100);
  });

  it('should parse custom BLS_PORT correctly', () => {
    setRequiredEnv();
    process.env['BLS_PORT'] = '8080';

    const config = loadBlsConfigFromEnv();

    expect(config.port).toBe(8080);
  });

  it('should throw ConfigError when BLS_PORT is 0', () => {
    setRequiredEnv();
    process.env['BLS_PORT'] = '0';

    expect(() => loadBlsConfigFromEnv()).toThrow(ConfigError);
    expect(() => loadBlsConfigFromEnv()).toThrow('must be an integer between 1 and 65535');
  });

  it('should throw ConfigError when BLS_PORT is 65536', () => {
    setRequiredEnv();
    process.env['BLS_PORT'] = '65536';

    expect(() => loadBlsConfigFromEnv()).toThrow(ConfigError);
    expect(() => loadBlsConfigFromEnv()).toThrow('must be an integer between 1 and 65535');
  });

  it('should throw ConfigError when BLS_PORT is non-numeric', () => {
    setRequiredEnv();
    process.env['BLS_PORT'] = 'not-a-port';

    expect(() => loadBlsConfigFromEnv()).toThrow(ConfigError);
    expect(() => loadBlsConfigFromEnv()).toThrow('must be an integer between 1 and 65535');
  });

  // --- BASE_PRICE_PER_BYTE ---

  it('should use default basePricePerByte of 10 when not set', () => {
    setRequiredEnv();

    const config = loadBlsConfigFromEnv();

    expect(config.basePricePerByte).toBe(10n);
  });

  it('should parse BASE_PRICE_PER_BYTE (unprefixed) correctly', () => {
    setRequiredEnv();
    process.env['BASE_PRICE_PER_BYTE'] = '50';

    const config = loadBlsConfigFromEnv();

    expect(config.basePricePerByte).toBe(50n);
  });

  it('should prefer BLS_BASE_PRICE_PER_BYTE over unprefixed', () => {
    setRequiredEnv();
    process.env['BLS_BASE_PRICE_PER_BYTE'] = '100';
    process.env['BASE_PRICE_PER_BYTE'] = '50';

    const config = loadBlsConfigFromEnv();

    expect(config.basePricePerByte).toBe(100n);
  });

  // --- OWNER_PUBKEY ---

  it('should validate OWNER_PUBKEY format when provided (64-char hex)', () => {
    setRequiredEnv();
    process.env['OWNER_PUBKEY'] = TEST_PUBKEY;

    const config = loadBlsConfigFromEnv();

    expect(config.ownerPubkey).toBe(TEST_PUBKEY);
  });

  it('should normalize OWNER_PUBKEY to lowercase', () => {
    setRequiredEnv();
    process.env['OWNER_PUBKEY'] = TEST_PUBKEY.toUpperCase();

    const config = loadBlsConfigFromEnv();

    expect(config.ownerPubkey).toBe(TEST_PUBKEY);
  });

  it('should throw ConfigError for invalid OWNER_PUBKEY', () => {
    setRequiredEnv();
    process.env['OWNER_PUBKEY'] = 'too-short';

    expect(() => loadBlsConfigFromEnv()).toThrow(ConfigError);
    expect(() => loadBlsConfigFromEnv()).toThrow('must be a 64-character hex string');
  });

  // --- DATA_DIR ---

  it('should use default /data for DATA_DIR when not set', () => {
    setRequiredEnv();

    const config = loadBlsConfigFromEnv();

    expect(config.dataDir).toBe('/data');
  });

  it('should parse custom DATA_DIR', () => {
    setRequiredEnv();
    process.env['DATA_DIR'] = '/custom/path';

    const config = loadBlsConfigFromEnv();

    expect(config.dataDir).toBe('/custom/path');
  });

  // --- KIND_OVERRIDES ---

  it('should parse KIND_OVERRIDES JSON correctly (unprefixed)', () => {
    setRequiredEnv();
    process.env['KIND_OVERRIDES'] = '{"1":"5","30023":"100"}';

    const config = loadBlsConfigFromEnv();

    expect(config.kindOverrides).toBeDefined();
    expect(config.kindOverrides?.get(1)).toBe(5n);
    expect(config.kindOverrides?.get(30023)).toBe(100n);
  });

  it('should prefer BLS_KIND_OVERRIDES over unprefixed', () => {
    setRequiredEnv();
    process.env['BLS_KIND_OVERRIDES'] = '{"1":"10"}';
    process.env['KIND_OVERRIDES'] = '{"1":"5"}';

    const config = loadBlsConfigFromEnv();

    expect(config.kindOverrides?.get(1)).toBe(10n);
  });

  it('should throw on invalid KIND_OVERRIDES JSON', () => {
    setRequiredEnv();
    process.env['KIND_OVERRIDES'] = 'not valid json';

    expect(() => loadBlsConfigFromEnv()).toThrow(PricingError);
    expect(() => loadBlsConfigFromEnv()).toThrow('Invalid JSON in KIND_OVERRIDES');
  });

  // --- SPSP_MIN_PRICE ---

  it('should set spspMinPrice to 0n when SPSP_MIN_PRICE=0', () => {
    setRequiredEnv();
    process.env['SPSP_MIN_PRICE'] = '0';

    const config = loadBlsConfigFromEnv();

    expect(config.spspMinPrice).toBe(0n);
  });

  it('should set spspMinPrice to 100n when SPSP_MIN_PRICE=100', () => {
    setRequiredEnv();
    process.env['SPSP_MIN_PRICE'] = '100';

    const config = loadBlsConfigFromEnv();

    expect(config.spspMinPrice).toBe(100n);
  });

  it('should leave spspMinPrice undefined when SPSP_MIN_PRICE is not set', () => {
    setRequiredEnv();

    const config = loadBlsConfigFromEnv();

    expect(config.spspMinPrice).toBeUndefined();
  });

  it('should throw ConfigError when SPSP_MIN_PRICE is negative', () => {
    setRequiredEnv();
    process.env['SPSP_MIN_PRICE'] = '-1';

    expect(() => loadBlsConfigFromEnv()).toThrow(ConfigError);
    expect(() => loadBlsConfigFromEnv()).toThrow('must be non-negative');
  });

  it('should throw ConfigError when SPSP_MIN_PRICE is not a number', () => {
    setRequiredEnv();
    process.env['SPSP_MIN_PRICE'] = 'not-a-number';

    expect(() => loadBlsConfigFromEnv()).toThrow(ConfigError);
    expect(() => loadBlsConfigFromEnv()).toThrow('must be a valid integer');
  });
});
