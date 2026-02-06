import { readFileSync } from 'fs';
import type { PricingConfig } from './types.js';
import { PricingError } from './types.js';

/**
 * Load pricing configuration from environment variables.
 *
 * Environment variables:
 * - RELAY_BASE_PRICE_PER_BYTE: Base price per byte (default: "10")
 * - RELAY_KIND_OVERRIDES: JSON object mapping kind to price (optional)
 *   Format: {"1":"5","30023":"100"}
 *
 * @returns PricingConfig loaded from environment
 * @throws PricingError if env vars contain invalid values
 */
export function loadPricingConfigFromEnv(): PricingConfig {
  // Parse base price
  const basePriceStr = process.env['RELAY_BASE_PRICE_PER_BYTE'] ?? '10';
  let basePricePerByte: bigint;
  try {
    basePricePerByte = BigInt(basePriceStr);
  } catch {
    throw new PricingError(
      `Invalid RELAY_BASE_PRICE_PER_BYTE: "${basePriceStr}" is not a valid integer`,
      'INVALID_ENV_CONFIG'
    );
  }

  if (basePricePerByte < 0n) {
    throw new PricingError(
      `Invalid RELAY_BASE_PRICE_PER_BYTE: value must be non-negative`,
      'INVALID_ENV_CONFIG'
    );
  }

  // Parse kind overrides if present
  const kindOverridesStr = process.env['RELAY_KIND_OVERRIDES'];
  let kindOverrides: Map<number, bigint> | undefined;

  if (kindOverridesStr) {
    kindOverrides = parseKindOverridesJson(kindOverridesStr);
  }

  return {
    basePricePerByte,
    kindOverrides,
  };
}

/**
 * Load pricing configuration from a JSON file.
 *
 * File format:
 * {
 *   "basePricePerByte": "10",
 *   "kindOverrides": {
 *     "0": "0",
 *     "1": "5",
 *     "30023": "100"
 *   }
 * }
 *
 * @param path - Path to the JSON config file
 * @returns PricingConfig loaded from file
 * @throws PricingError if file cannot be read or contains invalid values
 */
export function loadPricingConfigFromFile(path: string): PricingConfig {
  let fileContent: string;
  try {
    fileContent = readFileSync(path, 'utf-8');
  } catch (error) {
    throw new PricingError(
      `Failed to read config file: ${error instanceof Error ? error.message : 'Unknown error'}`,
      'CONFIG_FILE_ERROR'
    );
  }

  let parsed: unknown;
  try {
    parsed = JSON.parse(fileContent);
  } catch {
    throw new PricingError(
      `Invalid JSON in config file: ${path}`,
      'INVALID_FILE_CONFIG'
    );
  }

  if (typeof parsed !== 'object' || parsed === null) {
    throw new PricingError(
      `Config file must contain a JSON object`,
      'INVALID_FILE_CONFIG'
    );
  }

  const config = parsed as Record<string, unknown>;

  // Parse base price
  const basePriceStr = config['basePricePerByte'];
  if (typeof basePriceStr !== 'string' && typeof basePriceStr !== 'number') {
    throw new PricingError(
      `Config file must contain "basePricePerByte" as string or number`,
      'INVALID_FILE_CONFIG'
    );
  }

  let basePricePerByte: bigint;
  try {
    basePricePerByte = BigInt(basePriceStr);
  } catch {
    throw new PricingError(
      `Invalid basePricePerByte: "${basePriceStr}" is not a valid integer`,
      'INVALID_FILE_CONFIG'
    );
  }

  if (basePricePerByte < 0n) {
    throw new PricingError(
      `basePricePerByte must be non-negative`,
      'INVALID_FILE_CONFIG'
    );
  }

  // Parse kind overrides if present
  let kindOverrides: Map<number, bigint> | undefined;
  const kindOverridesValue = config['kindOverrides'];
  if (kindOverridesValue !== undefined) {
    if (typeof kindOverridesValue !== 'object' || kindOverridesValue === null) {
      throw new PricingError(
        `kindOverrides must be an object`,
        'INVALID_FILE_CONFIG'
      );
    }

    kindOverrides = new Map();
    const overridesObj = kindOverridesValue as Record<string, unknown>;

    for (const [kindStr, priceValue] of Object.entries(overridesObj)) {
      const kind = parseInt(kindStr, 10);
      if (isNaN(kind)) {
        throw new PricingError(
          `Invalid kind in kindOverrides: "${kindStr}" is not a valid integer`,
          'INVALID_FILE_CONFIG'
        );
      }

      if (typeof priceValue !== 'string' && typeof priceValue !== 'number') {
        throw new PricingError(
          `Invalid price for kind ${kind}: must be string or number`,
          'INVALID_FILE_CONFIG'
        );
      }

      let price: bigint;
      try {
        price = BigInt(priceValue);
      } catch {
        throw new PricingError(
          `Invalid price for kind ${kind}: "${priceValue}" is not a valid integer`,
          'INVALID_FILE_CONFIG'
        );
      }

      if (price < 0n) {
        throw new PricingError(
          `Price for kind ${kind} must be non-negative`,
          'INVALID_FILE_CONFIG'
        );
      }

      kindOverrides.set(kind, price);
    }
  }

  return {
    basePricePerByte,
    kindOverrides,
  };
}

/**
 * Parse kind overrides JSON string to Map.
 */
function parseKindOverridesJson(jsonStr: string): Map<number, bigint> {
  let parsed: unknown;
  try {
    parsed = JSON.parse(jsonStr);
  } catch {
    throw new PricingError(
      `Invalid JSON in RELAY_KIND_OVERRIDES: ${jsonStr}`,
      'INVALID_ENV_CONFIG'
    );
  }

  if (typeof parsed !== 'object' || parsed === null) {
    throw new PricingError(
      `RELAY_KIND_OVERRIDES must be a JSON object`,
      'INVALID_ENV_CONFIG'
    );
  }

  const result = new Map<number, bigint>();
  const obj = parsed as Record<string, unknown>;

  for (const [kindStr, priceValue] of Object.entries(obj)) {
    const kind = parseInt(kindStr, 10);
    if (isNaN(kind)) {
      throw new PricingError(
        `Invalid kind in RELAY_KIND_OVERRIDES: "${kindStr}" is not a valid integer`,
        'INVALID_ENV_CONFIG'
      );
    }

    if (typeof priceValue !== 'string' && typeof priceValue !== 'number') {
      throw new PricingError(
        `Invalid price for kind ${kind} in RELAY_KIND_OVERRIDES: must be string or number`,
        'INVALID_ENV_CONFIG'
      );
    }

    let price: bigint;
    try {
      price = BigInt(priceValue);
    } catch {
      throw new PricingError(
        `Invalid price for kind ${kind} in RELAY_KIND_OVERRIDES: "${priceValue}" is not a valid integer`,
        'INVALID_ENV_CONFIG'
      );
    }

    if (price < 0n) {
      throw new PricingError(
        `Price for kind ${kind} in RELAY_KIND_OVERRIDES must be non-negative`,
        'INVALID_ENV_CONFIG'
      );
    }

    result.set(kind, price);
  }

  return result;
}
