/**
 * Health response for the relay's HTTP server.
 *
 * The relay is a plain read/write app (no payment, connector, or settlement
 * layer), so the health response is deliberately minimal: liveness plus the
 * node's identity and version. It is served from `GET /health` on the write
 * port and is the target of the container healthcheck.
 *
 * @module
 */

import { VERSION } from '../version.js';

/** Configuration for building a health response. */
export interface HealthConfig {
  /** Node's Nostr pubkey (64-char hex). */
  pubkey: string;
}

/** The health response shape. */
export interface HealthResponse {
  status: 'healthy';
  pubkey: string;
  capabilities: string[];
  version: string;
  timestamp: number;
}

/**
 * Build a health response for the relay.
 *
 * Pure function: takes a config and returns the response object, so it is
 * trivially unit-testable and reusable.
 *
 * @param config - Health configuration (the node's pubkey).
 * @returns The health response object.
 */
export function createHealthResponse(config: HealthConfig): HealthResponse {
  return {
    status: 'healthy',
    pubkey: config.pubkey,
    capabilities: ['relay'],
    version: VERSION,
    timestamp: Date.now(),
  };
}
