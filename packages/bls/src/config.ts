import { getPublicKey } from 'nostr-tools/pure';
import { ConfigError } from './errors.js';
import { loadPricingConfigFromEnv } from './pricing/config.js';
import type { PricingConfig } from './pricing/types.js';

/**
 * Parsed and validated BLS environment configuration.
 */
export interface BlsEnvConfig {
  /** Unique node identifier */
  nodeId: string;
  /** Hex-encoded Nostr secret key (64 lowercase hex chars) */
  nostrSecretKey: string;
  /** Derived Nostr public key */
  pubkey: string;
  /** Node's ILP address */
  ilpAddress: string;
  /** HTTP port (1–65535) */
  port: number;
  /** Base price per byte for event storage */
  basePricePerByte: bigint;
  /** Optional owner pubkey for self-write bypass */
  ownerPubkey?: string;
  /** Directory for SQLite database */
  dataDir: string;
  /** Optional kind-specific price overrides */
  kindOverrides?: Map<number, bigint>;
  /** Optional minimum price for SPSP request events (kind:23194) */
  spspMinPrice?: bigint;
  /** Optional Forgejo URL for NIP-34 Git integration */
  forgejoUrl?: string;
  /** Optional Forgejo API token for NIP-34 Git integration */
  forgejoToken?: string;
  /** Optional Forgejo owner/organization for repositories */
  forgejoOwner?: string;
}

const HEX_64_REGEX = /^[0-9a-f]{64}$/;
const ILP_ADDRESS_REGEX = /^g\.[a-zA-Z0-9.-]+$/;

/**
 * Load and validate all BLS configuration from environment variables.
 *
 * Required:
 * - NODE_ID: Unique node identifier
 * - NOSTR_SECRET_KEY: 64-char hex Nostr secret key
 * - ILP_ADDRESS: ILP address (starts with g., alphanumeric/dots/hyphens)
 *
 * Optional:
 * - BLS_PORT: HTTP port (default: 3100)
 * - BLS_BASE_PRICE_PER_BYTE / RELAY_BASE_PRICE_PER_BYTE / BASE_PRICE_PER_BYTE: Pricing rate (default: 10)
 * - OWNER_PUBKEY: Pubkey for self-write bypass
 * - DATA_DIR: SQLite data directory (default: /data)
 * - BLS_KIND_OVERRIDES / RELAY_KIND_OVERRIDES / KIND_OVERRIDES: JSON kind→price map
 *
 * @returns Validated BlsEnvConfig
 * @throws ConfigError if any required variable is missing or invalid
 */
export function loadBlsConfigFromEnv(): BlsEnvConfig {
  // Required: NODE_ID
  const nodeId = requireEnv('NODE_ID');

  // Required: NOSTR_SECRET_KEY (64 hex chars)
  const rawSecretKey = requireEnv('NOSTR_SECRET_KEY');
  const nostrSecretKey = rawSecretKey.toLowerCase();
  if (!HEX_64_REGEX.test(nostrSecretKey)) {
    throw new ConfigError(
      'NOSTR_SECRET_KEY',
      `must be a 64-character hex string, got ${rawSecretKey.length} characters`
    );
  }

  // Derive pubkey from secret key (regex validated above guarantees 64 hex chars)
  const hexPairs = nostrSecretKey.match(/.{2}/g) as RegExpMatchArray;
  const secretKeyBytes = Uint8Array.from(
    hexPairs.map((byte) => parseInt(byte, 16))
  );
  const pubkey = getPublicKey(secretKeyBytes);

  // Required: ILP_ADDRESS
  const ilpAddress = requireEnv('ILP_ADDRESS');
  if (!ILP_ADDRESS_REGEX.test(ilpAddress)) {
    throw new ConfigError(
      'ILP_ADDRESS',
      `must start with "g." and contain only alphanumeric characters, dots, and hyphens, got "${ilpAddress}"`
    );
  }

  // Optional: BLS_PORT (default 3100)
  const port = parsePort();

  // Optional: OWNER_PUBKEY (64-char lowercase hex)
  const rawOwnerPubkey = process.env['OWNER_PUBKEY'];
  let ownerPubkey: string | undefined;
  if (rawOwnerPubkey) {
    ownerPubkey = rawOwnerPubkey.toLowerCase();
    if (!HEX_64_REGEX.test(ownerPubkey)) {
      throw new ConfigError(
        'OWNER_PUBKEY',
        `must be a 64-character hex string, got ${rawOwnerPubkey.length} characters`
      );
    }
  }

  // Optional: DATA_DIR (default /data)
  const dataDir = process.env['DATA_DIR'] ?? '/data';

  // Optional: SPSP_MIN_PRICE (bigint, non-negative)
  let spspMinPrice: bigint | undefined;
  const rawSpspMinPrice = process.env['SPSP_MIN_PRICE'];
  if (rawSpspMinPrice !== undefined && rawSpspMinPrice !== '') {
    try {
      spspMinPrice = BigInt(rawSpspMinPrice);
    } catch {
      throw new ConfigError(
        'SPSP_MIN_PRICE',
        `must be a valid integer, got "${rawSpspMinPrice}"`
      );
    }
    if (spspMinPrice < 0n) {
      throw new ConfigError(
        'SPSP_MIN_PRICE',
        `must be non-negative, got "${rawSpspMinPrice}"`
      );
    }
  }

  // Pricing: delegate to loadPricingConfigFromEnv() with unprefixed fallbacks
  propagateUnprefixedEnvVars();
  const pricingConfig: PricingConfig = loadPricingConfigFromEnv();

  // Optional: NIP-34 Forgejo Git integration
  const forgejoUrl = process.env['FORGEJO_URL'];
  const forgejoToken = process.env['FORGEJO_TOKEN'];
  const forgejoOwner = process.env['FORGEJO_OWNER'];

  return {
    nodeId,
    nostrSecretKey,
    pubkey,
    ilpAddress,
    port,
    basePricePerByte: pricingConfig.basePricePerByte,
    ownerPubkey,
    dataDir,
    kindOverrides: pricingConfig.kindOverrides,
    spspMinPrice,
    forgejoUrl,
    forgejoToken,
    forgejoOwner,
  };
}

/**
 * Read a required environment variable or throw ConfigError.
 */
function requireEnv(name: string): string {
  const value = process.env[name];
  if (!value) {
    throw new ConfigError(name, 'is required but not set');
  }
  return value;
}

/**
 * Parse and validate BLS_PORT. Default 3100, must be 1–65535.
 */
function parsePort(): number {
  const portStr = process.env['BLS_PORT'];
  if (!portStr) return 3100;

  const port = parseInt(portStr, 10);
  if (isNaN(port) || port < 1 || port > 65535) {
    throw new ConfigError(
      'BLS_PORT',
      `must be an integer between 1 and 65535, got "${portStr}"`
    );
  }
  return port;
}

/**
 * If unprefixed BASE_PRICE_PER_BYTE or KIND_OVERRIDES are set
 * but neither BLS_ nor RELAY_ prefixed versions exist,
 * propagate them so loadPricingConfigFromEnv() picks them up.
 */
function propagateUnprefixedEnvVars(): void {
  if (
    process.env['BASE_PRICE_PER_BYTE'] &&
    !process.env['BLS_BASE_PRICE_PER_BYTE'] &&
    !process.env['RELAY_BASE_PRICE_PER_BYTE']
  ) {
    process.env['BLS_BASE_PRICE_PER_BYTE'] = process.env['BASE_PRICE_PER_BYTE'];
  }

  if (
    process.env['KIND_OVERRIDES'] &&
    !process.env['BLS_KIND_OVERRIDES'] &&
    !process.env['RELAY_KIND_OVERRIDES']
  ) {
    process.env['BLS_KIND_OVERRIDES'] = process.env['KIND_OVERRIDES'];
  }
}
