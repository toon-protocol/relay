/**
 * Tests for the relay health response.
 *
 * The relay is a plain read/write app, so `/health` is minimal: liveness,
 * identity (pubkey), capabilities, version, and a timestamp.
 */

import { describe, it, expect } from 'vitest';
import { createHealthResponse, type HealthConfig } from './health.js';
import { VERSION } from '../version.js';

const PUBKEY = 'a'.repeat(64); // 64-char hex placeholder

function makeConfig(overrides: Partial<HealthConfig> = {}): HealthConfig {
  return { pubkey: PUBKEY, ...overrides };
}

describe('createHealthResponse', () => {
  it('reports healthy with identity, capabilities, version, and timestamp', () => {
    const res = createHealthResponse(makeConfig());

    expect(res.status).toBe('healthy');
    expect(res.pubkey).toBe(PUBKEY);
    expect(res.capabilities).toEqual(['relay']);
    expect(res.version).toBe(VERSION);
    expect(typeof res.timestamp).toBe('number');
  });

  it('echoes the provided pubkey', () => {
    const pubkey = 'b'.repeat(64);
    expect(createHealthResponse(makeConfig({ pubkey })).pubkey).toBe(pubkey);
  });

  it('does not leak any payment / connector fields', () => {
    const res = createHealthResponse(makeConfig()) as unknown as Record<
      string,
      unknown
    >;
    for (const field of [
      'x402',
      'pricing',
      'chain',
      'ilpAddress',
      'peerCount',
      'channelCount',
      'tee',
      'phase',
    ]) {
      expect(res[field]).toBeUndefined();
    }
  });
});
