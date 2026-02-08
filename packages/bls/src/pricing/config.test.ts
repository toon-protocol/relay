import { describe, it, expect, beforeEach, afterEach, vi } from 'vitest';
import { writeFileSync, unlinkSync, mkdtempSync } from 'fs';
import { join } from 'path';
import { tmpdir } from 'os';
import { loadPricingConfigFromEnv, loadPricingConfigFromFile } from './config.js';
import { PricingError } from './types.js';

describe('loadPricingConfigFromEnv', () => {
  const originalEnv = process.env;

  beforeEach(() => {
    vi.resetModules();
    process.env = { ...originalEnv };
  });

  afterEach(() => {
    process.env = originalEnv;
  });

  it('should use defaults when no env vars set', () => {
    delete process.env['BLS_BASE_PRICE_PER_BYTE'];
    delete process.env['RELAY_BASE_PRICE_PER_BYTE'];
    delete process.env['BLS_KIND_OVERRIDES'];
    delete process.env['RELAY_KIND_OVERRIDES'];

    const config = loadPricingConfigFromEnv();
    expect(config.basePricePerByte).toBe(10n);
    expect(config.kindOverrides).toBeUndefined();
  });

  it('should parse BLS_BASE_PRICE_PER_BYTE', () => {
    process.env['BLS_BASE_PRICE_PER_BYTE'] = '50';

    const config = loadPricingConfigFromEnv();
    expect(config.basePricePerByte).toBe(50n);
  });

  it('should fall back to RELAY_BASE_PRICE_PER_BYTE', () => {
    delete process.env['BLS_BASE_PRICE_PER_BYTE'];
    process.env['RELAY_BASE_PRICE_PER_BYTE'] = '75';

    const config = loadPricingConfigFromEnv();
    expect(config.basePricePerByte).toBe(75n);
  });

  it('should prefer BLS_ prefix over RELAY_ prefix', () => {
    process.env['BLS_BASE_PRICE_PER_BYTE'] = '50';
    process.env['RELAY_BASE_PRICE_PER_BYTE'] = '75';

    const config = loadPricingConfigFromEnv();
    expect(config.basePricePerByte).toBe(50n);
  });

  it('should parse large BLS_BASE_PRICE_PER_BYTE', () => {
    process.env['BLS_BASE_PRICE_PER_BYTE'] = '999999999999999999999';

    const config = loadPricingConfigFromEnv();
    expect(config.basePricePerByte).toBe(999999999999999999999n);
  });

  it('should parse kind overrides from BLS_KIND_OVERRIDES', () => {
    process.env['BLS_KIND_OVERRIDES'] = '{"0":"0","30023":"100"}';

    const config = loadPricingConfigFromEnv();
    expect(config.kindOverrides).toBeDefined();
    expect(config.kindOverrides?.get(0)).toBe(0n);
    expect(config.kindOverrides?.get(30023)).toBe(100n);
  });

  it('should fall back to RELAY_KIND_OVERRIDES', () => {
    delete process.env['BLS_KIND_OVERRIDES'];
    process.env['RELAY_KIND_OVERRIDES'] = '{"1":"5"}';

    const config = loadPricingConfigFromEnv();
    expect(config.kindOverrides?.get(1)).toBe(5n);
  });

  it('should prefer BLS_KIND_OVERRIDES over RELAY_KIND_OVERRIDES', () => {
    process.env['BLS_KIND_OVERRIDES'] = '{"1":"10"}';
    process.env['RELAY_KIND_OVERRIDES'] = '{"1":"5"}';

    const config = loadPricingConfigFromEnv();
    expect(config.kindOverrides?.get(1)).toBe(10n);
  });

  it('should parse kind overrides with numeric values', () => {
    process.env['BLS_KIND_OVERRIDES'] = '{"1":5,"30023":100}';

    const config = loadPricingConfigFromEnv();
    expect(config.kindOverrides?.get(1)).toBe(5n);
    expect(config.kindOverrides?.get(30023)).toBe(100n);
  });

  it('should throw PricingError for invalid JSON in kind overrides', () => {
    process.env['BLS_KIND_OVERRIDES'] = 'not valid json';

    expect(() => loadPricingConfigFromEnv()).toThrowError(PricingError);
    expect(() => loadPricingConfigFromEnv()).toThrowError(
      'Invalid JSON in KIND_OVERRIDES'
    );
  });

  it('should throw PricingError for non-object kind overrides', () => {
    process.env['BLS_KIND_OVERRIDES'] = '"string"';

    expect(() => loadPricingConfigFromEnv()).toThrowError(PricingError);
    expect(() => loadPricingConfigFromEnv()).toThrowError(
      'KIND_OVERRIDES must be a JSON object'
    );
  });

  it('should throw PricingError for invalid base price', () => {
    process.env['BLS_BASE_PRICE_PER_BYTE'] = 'not-a-number';

    expect(() => loadPricingConfigFromEnv()).toThrowError(PricingError);
    expect(() => loadPricingConfigFromEnv()).toThrowError(
      'Invalid BASE_PRICE_PER_BYTE'
    );
  });

  it('should throw PricingError for negative base price', () => {
    process.env['BLS_BASE_PRICE_PER_BYTE'] = '-10';

    expect(() => loadPricingConfigFromEnv()).toThrowError(PricingError);
    expect(() => loadPricingConfigFromEnv()).toThrowError(
      'value must be non-negative'
    );
  });

  it('should throw PricingError for negative kind override', () => {
    process.env['BLS_KIND_OVERRIDES'] = '{"1":"-5"}';

    expect(() => loadPricingConfigFromEnv()).toThrowError(PricingError);
    expect(() => loadPricingConfigFromEnv()).toThrowError(
      'must be non-negative'
    );
  });

  it('should throw PricingError for invalid kind in overrides', () => {
    process.env['BLS_KIND_OVERRIDES'] = '{"notANumber":"5"}';

    expect(() => loadPricingConfigFromEnv()).toThrowError(PricingError);
    expect(() => loadPricingConfigFromEnv()).toThrowError(
      'is not a valid integer'
    );
  });

  it('should throw PricingError for invalid price value in overrides', () => {
    process.env['BLS_KIND_OVERRIDES'] = '{"1":"not-a-number"}';

    expect(() => loadPricingConfigFromEnv()).toThrowError(PricingError);
    expect(() => loadPricingConfigFromEnv()).toThrowError(
      'is not a valid integer'
    );
  });
});

describe('loadPricingConfigFromFile', () => {
  let tempDir: string;

  beforeEach(() => {
    tempDir = mkdtempSync(join(tmpdir(), 'pricing-test-'));
  });

  afterEach(() => {
    // Clean up temp files
    try {
      const files = ['valid.json', 'invalid.json', 'negative.json', 'missing-base.json'];
      for (const file of files) {
        try {
          unlinkSync(join(tempDir, file));
        } catch {
          // Ignore if file doesn't exist
        }
      }
    } catch {
      // Ignore cleanup errors
    }
  });

  it('should load valid config file', () => {
    const configPath = join(tempDir, 'valid.json');
    writeFileSync(
      configPath,
      JSON.stringify({
        basePricePerByte: '10',
        kindOverrides: {
          '0': '0',
          '1': '5',
          '30023': '100',
        },
      })
    );

    const config = loadPricingConfigFromFile(configPath);
    expect(config.basePricePerByte).toBe(10n);
    expect(config.kindOverrides?.get(0)).toBe(0n);
    expect(config.kindOverrides?.get(1)).toBe(5n);
    expect(config.kindOverrides?.get(30023)).toBe(100n);
  });

  it('should load config with numeric basePricePerByte', () => {
    const configPath = join(tempDir, 'valid.json');
    writeFileSync(
      configPath,
      JSON.stringify({
        basePricePerByte: 25,
      })
    );

    const config = loadPricingConfigFromFile(configPath);
    expect(config.basePricePerByte).toBe(25n);
  });

  it('should load config without kindOverrides', () => {
    const configPath = join(tempDir, 'valid.json');
    writeFileSync(
      configPath,
      JSON.stringify({
        basePricePerByte: '10',
      })
    );

    const config = loadPricingConfigFromFile(configPath);
    expect(config.basePricePerByte).toBe(10n);
    expect(config.kindOverrides).toBeUndefined();
  });

  it('should throw PricingError for missing file', () => {
    const configPath = join(tempDir, 'nonexistent.json');

    expect(() => loadPricingConfigFromFile(configPath)).toThrowError(
      PricingError
    );
    expect(() => loadPricingConfigFromFile(configPath)).toThrowError(
      'Failed to read config file'
    );
  });

  it('should throw PricingError for invalid JSON', () => {
    const configPath = join(tempDir, 'invalid.json');
    writeFileSync(configPath, 'not valid json {');

    expect(() => loadPricingConfigFromFile(configPath)).toThrowError(
      PricingError
    );
    expect(() => loadPricingConfigFromFile(configPath)).toThrowError(
      'Invalid JSON in config file'
    );
  });

  it('should throw PricingError for negative basePricePerByte', () => {
    const configPath = join(tempDir, 'negative.json');
    writeFileSync(
      configPath,
      JSON.stringify({
        basePricePerByte: '-10',
      })
    );

    expect(() => loadPricingConfigFromFile(configPath)).toThrowError(
      PricingError
    );
    expect(() => loadPricingConfigFromFile(configPath)).toThrowError(
      'basePricePerByte must be non-negative'
    );
  });

  it('should throw PricingError for negative kind override', () => {
    const configPath = join(tempDir, 'negative.json');
    writeFileSync(
      configPath,
      JSON.stringify({
        basePricePerByte: '10',
        kindOverrides: {
          '1': '-5',
        },
      })
    );

    expect(() => loadPricingConfigFromFile(configPath)).toThrowError(
      PricingError
    );
    expect(() => loadPricingConfigFromFile(configPath)).toThrowError(
      'must be non-negative'
    );
  });

  it('should throw PricingError for missing basePricePerByte', () => {
    const configPath = join(tempDir, 'missing-base.json');
    writeFileSync(
      configPath,
      JSON.stringify({
        kindOverrides: { '1': '5' },
      })
    );

    expect(() => loadPricingConfigFromFile(configPath)).toThrowError(
      PricingError
    );
    expect(() => loadPricingConfigFromFile(configPath)).toThrowError(
      'must contain "basePricePerByte"'
    );
  });

  it('should throw PricingError for non-object kindOverrides', () => {
    const configPath = join(tempDir, 'invalid.json');
    writeFileSync(
      configPath,
      JSON.stringify({
        basePricePerByte: '10',
        kindOverrides: 'not an object',
      })
    );

    expect(() => loadPricingConfigFromFile(configPath)).toThrowError(
      PricingError
    );
    expect(() => loadPricingConfigFromFile(configPath)).toThrowError(
      'kindOverrides must be an object'
    );
  });

  it('should throw PricingError for invalid kind in file', () => {
    const configPath = join(tempDir, 'invalid.json');
    writeFileSync(
      configPath,
      JSON.stringify({
        basePricePerByte: '10',
        kindOverrides: {
          notANumber: '5',
        },
      })
    );

    expect(() => loadPricingConfigFromFile(configPath)).toThrowError(
      PricingError
    );
    expect(() => loadPricingConfigFromFile(configPath)).toThrowError(
      'is not a valid integer'
    );
  });
});
