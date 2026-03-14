/**
 * Enriched health response for Crosstown relay nodes (Story 3.6).
 *
 * Provides a pure function `createHealthResponse()` that builds a comprehensive
 * health JSON object combining static configuration (pricing, chain, version,
 * capabilities) with live runtime state (phase, peerCount, channelCount).
 *
 * The response mirrors kind:10035 service discovery event fields but adds
 * runtime-only fields that cannot be known at event publish time.
 *
 * @module
 */

import { VERSION } from '@crosstown/core';
import type { BootstrapPhase } from '@crosstown/core';

/** Configuration for building a health response. */
export interface HealthConfig {
  /** Current bootstrap phase. */
  phase: BootstrapPhase;
  /** Node's Nostr pubkey (64-char hex). */
  pubkey: string;
  /** Node's ILP address. */
  ilpAddress: string;
  /** Number of registered peers. */
  peerCount: number;
  /** Number of discovered (not yet registered) peers. */
  discoveredPeerCount: number;
  /** Number of open payment channels. */
  channelCount: number;
  /**
   * Base price per byte (bigint from config, converted to number via Number()).
   * Values exceeding Number.MAX_SAFE_INTEGER (2^53 - 1) will lose precision.
   */
  basePricePerByte: bigint;
  /** Whether x402 is enabled. */
  x402Enabled: boolean;
  /** Chain preset name. */
  chain: string;
}

/** The enriched health response shape. */
export interface HealthResponse {
  status: 'healthy';
  phase: BootstrapPhase;
  pubkey: string;
  ilpAddress: string;
  peerCount: number;
  discoveredPeerCount: number;
  channelCount: number;
  pricing: {
    basePricePerByte: number;
    currency: 'USDC';
  };
  x402?: {
    enabled: true;
    endpoint: string;
  };
  capabilities: string[];
  chain: string;
  version: string;
  sdk: true;
  timestamp: number;
}

/**
 * Build an enriched health response from the given configuration.
 *
 * This is a pure function -- it takes a config object and returns a response
 * object. No Hono context or HTTP request is needed, making it easy to unit
 * test and reuse across entrypoints.
 *
 * The `x402` field is entirely omitted when x402 is disabled (AC #2).
 * This matches the same omission semantics used in kind:10035 events.
 *
 * @param config - Health configuration with runtime state and static config.
 * @returns The enriched health response object.
 */
export function createHealthResponse(config: HealthConfig): HealthResponse {
  const response: HealthResponse = {
    status: 'healthy',
    phase: config.phase,
    pubkey: config.pubkey,
    ilpAddress: config.ilpAddress,
    peerCount: config.peerCount,
    discoveredPeerCount: config.discoveredPeerCount,
    channelCount: config.channelCount,
    pricing: {
      basePricePerByte: Number(config.basePricePerByte),
      currency: 'USDC',
    },
    capabilities: config.x402Enabled ? ['relay', 'x402'] : ['relay'],
    chain: config.chain,
    version: VERSION,
    sdk: true,
    timestamp: Date.now(),
  };

  if (config.x402Enabled) {
    response.x402 = {
      enabled: true,
      endpoint: '/publish',
    };
  }

  return response;
}
