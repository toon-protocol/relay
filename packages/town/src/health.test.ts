/**
 * ATDD tests for Story 3.6: Enriched /health Endpoint (FR-PROD-6)
 *
 * Validates:
 * - /health response schema (snapshot test)
 * - /health reflects live node state (peerCount, channelCount)
 * - createHealthResponse() pure function behavior
 *
 * Test IDs from test-design-epic-3.md:
 * - 3.6-UNIT-001 [P2]: /health response schema
 * - 3.6-INT-001 [P2]: /health reflects live state
 *
 * ATDD stubs corrected per enriched story review:
 * - Factory rewritten to match HealthConfig interface (added phase, pubkey,
 *   ilpAddress; removed currency, x402Endpoint, capabilities, version;
 *   basePricePerByte changed to bigint)
 * - Import name: createHealthResponse (not createHealthHandler)
 * - x402 disabled test: asserts x402 field entirely omitted (AC #2)
 * - BootstrapPhase regex: discovering|registering|announcing|ready|failed
 */

import { describe, it, expect } from 'vitest';
import { createHealthResponse, type HealthConfig } from './health.js';
import { VERSION } from '@crosstown/core';

// ============================================================================
// Factories
// ============================================================================

/**
 * Creates a mock HealthConfig with sensible defaults.
 *
 * Fields match the HealthConfig interface exactly:
 * - phase, pubkey, ilpAddress, peerCount, discoveredPeerCount,
 *   channelCount, basePricePerByte (bigint), x402Enabled, chain
 *
 * Derived fields (currency, capabilities, version, x402.endpoint)
 * are NOT part of HealthConfig -- createHealthResponse() derives them.
 */
function _createHealthConfig(
  overrides: Partial<HealthConfig> = {}
): HealthConfig {
  return {
    phase: 'ready',
    pubkey: 'abcdef1234567890abcdef1234567890abcdef1234567890abcdef1234567890',
    ilpAddress: 'g.crosstown.abcdef12345678',
    peerCount: 5,
    discoveredPeerCount: 12,
    channelCount: 3,
    basePricePerByte: 10n,
    x402Enabled: true,
    chain: 'arbitrum-one',
    ...overrides,
  };
}

// ============================================================================
// Tests
// ============================================================================

describe('Story 3.6: Enriched /health Endpoint', () => {
  // --------------------------------------------------------------------------
  // 3.6-UNIT-001 [P2]: /health response schema
  // Risk: E3-R013
  // --------------------------------------------------------------------------
  describe('/health response schema (3.6-UNIT-001)', () => {
    it('[P2] response includes phase, peerCount, channelCount, pricing, x402, capabilities, chain, version (T-3.6-01)', () => {
      // Arrange
      const config = _createHealthConfig();

      // Act
      const response = createHealthResponse(config);

      // Assert -- snapshot test for schema stability
      expect(response).toMatchObject({
        status: 'healthy',
        phase: expect.stringMatching(
          /^(discovering|registering|announcing|ready|failed)$/
        ),
        pubkey: expect.stringMatching(/^[a-f0-9]{64}$/),
        ilpAddress: expect.any(String),
        peerCount: expect.any(Number),
        discoveredPeerCount: expect.any(Number),
        channelCount: expect.any(Number),
        pricing: {
          basePricePerByte: expect.any(Number),
          currency: 'USDC',
        },
        x402: {
          enabled: true,
          endpoint: '/publish',
        },
        capabilities: expect.arrayContaining(['relay', 'x402']),
        chain: expect.any(String),
        version: expect.stringMatching(/^\d+\.\d+\.\d+/),
        sdk: true,
        timestamp: expect.any(Number),
      });
    });

    it('[P2] response with x402 disabled omits x402 field entirely (T-3.6-02)', () => {
      // Arrange
      const config = _createHealthConfig({ x402Enabled: false });

      // Act
      const response = createHealthResponse(config);

      // Assert -- AC #2: x402 field entirely omitted when disabled
      expect(response.x402).toBeUndefined();
      expect('x402' in response).toBe(false);
      expect(response.capabilities).not.toContain('x402');
      expect(response.capabilities).toContain('relay');
    });
  });

  // --------------------------------------------------------------------------
  // 3.6-INT-001 [P2]: /health reflects live state
  // --------------------------------------------------------------------------
  describe('/health reflects live state (3.6-INT-001)', () => {
    it('[P2] peerCount and channelCount match actual node state (T-3.6-03)', () => {
      // Arrange
      const config = _createHealthConfig({
        peerCount: 3,
        channelCount: 2,
        discoveredPeerCount: 7,
      });

      // Act
      const response = createHealthResponse(config);

      // Assert -- response reflects the exact counts from config
      expect(response.peerCount).toBe(3);
      expect(response.channelCount).toBe(2);
      expect(response.discoveredPeerCount).toBe(7);
    });
  });

  // --------------------------------------------------------------------------
  // Additional unit tests for createHealthResponse() pure function
  // --------------------------------------------------------------------------
  describe('createHealthResponse() pure function', () => {
    it('[P2] returns correct version from @crosstown/core (T-3.6-04)', () => {
      // Arrange
      const config = _createHealthConfig();

      // Act
      const response = createHealthResponse(config);

      // Assert -- version must match the VERSION constant from @crosstown/core
      expect(response.version).toBe(VERSION);
      expect(response.version).toMatch(/^\d+\.\d+\.\d+/);
    });

    it('[P2] returns sdk: true (T-3.6-05)', () => {
      // Arrange
      const config = _createHealthConfig();

      // Act
      const response = createHealthResponse(config);

      // Assert -- backward compatibility: sdk must be true
      expect(response.sdk).toBe(true);
    });

    it('[P2] always returns status healthy (T-3.6-06)', () => {
      // Arrange
      const config = _createHealthConfig({ phase: 'discovering' });

      // Act
      const response = createHealthResponse(config);

      // Assert -- status is always 'healthy' regardless of phase
      expect(response.status).toBe('healthy');
    });

    it('[P2] timestamp is a recent number (T-3.6-07)', () => {
      // Arrange
      const config = _createHealthConfig();
      const before = Date.now();

      // Act
      const response = createHealthResponse(config);

      // Assert -- timestamp should be within 5 seconds of now
      const after = Date.now();
      expect(response.timestamp).toBeGreaterThanOrEqual(before);
      expect(response.timestamp).toBeLessThanOrEqual(after);
      expect(typeof response.timestamp).toBe('number');
    });

    it('[P2] includes pubkey and ilpAddress from config (T-3.6-08)', () => {
      // Arrange
      const pubkey = 'ff'.repeat(32);
      const ilpAddress = 'g.crosstown.testnode';
      const config = _createHealthConfig({ pubkey, ilpAddress });

      // Act
      const response = createHealthResponse(config);

      // Assert -- pubkey and ilpAddress come directly from config
      expect(response.pubkey).toBe(pubkey);
      expect(response.ilpAddress).toBe(ilpAddress);
    });

    it('[P2] x402 enabled includes endpoint /publish (T-3.6-09)', () => {
      // Arrange
      const config = _createHealthConfig({ x402Enabled: true });

      // Act
      const response = createHealthResponse(config);

      // Assert -- when x402 enabled, endpoint is always /publish
      expect(response.x402).toBeDefined();
      expect(response.x402!.enabled).toBe(true);
      expect(response.x402!.endpoint).toBe('/publish');
    });

    it('[P2] capabilities array always includes relay (T-3.6-10)', () => {
      // Arrange
      const configEnabled = _createHealthConfig({ x402Enabled: true });
      const configDisabled = _createHealthConfig({ x402Enabled: false });

      // Act
      const responseEnabled = createHealthResponse(configEnabled);
      const responseDisabled = createHealthResponse(configDisabled);

      // Assert -- 'relay' is always present in capabilities
      expect(responseEnabled.capabilities).toContain('relay');
      expect(responseDisabled.capabilities).toContain('relay');
      expect(responseEnabled.capabilities).toContain('x402');
      expect(responseDisabled.capabilities).not.toContain('x402');
    });

    it('[P2] pricing basePricePerByte converted from bigint (T-3.6-11)', () => {
      // Arrange
      const config = _createHealthConfig({ basePricePerByte: 42n });

      // Act
      const response = createHealthResponse(config);

      // Assert -- bigint converted to number via Number()
      expect(response.pricing.basePricePerByte).toBe(42);
      expect(typeof response.pricing.basePricePerByte).toBe('number');
      expect(response.pricing.currency).toBe('USDC');
    });
  });

  // --------------------------------------------------------------------------
  // Gap-fill tests: coverage for AC edge cases not in original test IDs
  // --------------------------------------------------------------------------
  describe('AC gap-fill: config passthrough and schema strictness', () => {
    it('[P2] chain field passes through from config (AC #1)', () => {
      // Arrange -- use a specific chain value
      const config = _createHealthConfig({ chain: 'arbitrum-sepolia' });

      // Act
      const response = createHealthResponse(config);

      // Assert -- chain in response matches the exact config value
      expect(response.chain).toBe('arbitrum-sepolia');
    });

    it('[P2] phase field passes through from config (AC #1)', () => {
      // Arrange -- use each valid BootstrapPhase value
      const phases = [
        'discovering',
        'registering',
        'announcing',
        'ready',
        'failed',
      ] as const;

      for (const phase of phases) {
        const config = _createHealthConfig({ phase });

        // Act
        const response = createHealthResponse(config);

        // Assert -- phase in response matches the exact config value
        expect(response.phase).toBe(phase);
      }
    });

    it('[P2] all fields present during non-ready bootstrap phases (AC #1)', () => {
      // Arrange -- node is still discovering, peerCount/channelCount are 0
      const config = _createHealthConfig({
        phase: 'discovering',
        peerCount: 0,
        discoveredPeerCount: 0,
        channelCount: 0,
      });

      // Act
      const response = createHealthResponse(config);

      // Assert -- all fields are present even during bootstrap
      expect(response.status).toBe('healthy');
      expect(response.phase).toBe('discovering');
      expect(response.peerCount).toBe(0);
      expect(response.discoveredPeerCount).toBe(0);
      expect(response.channelCount).toBe(0);
      expect(response.pricing).toBeDefined();
      expect(response.capabilities).toBeDefined();
      expect(response.chain).toBeDefined();
      expect(response.version).toBeDefined();
      expect(response.sdk).toBe(true);
      expect(response.timestamp).toBeDefined();
      expect(response.pubkey).toBeDefined();
      expect(response.ilpAddress).toBeDefined();
    });

    it('[P2] response has exactly the expected keys when x402 enabled (schema strictness)', () => {
      // Arrange
      const config = _createHealthConfig({ x402Enabled: true });

      // Act
      const response = createHealthResponse(config);

      // Assert -- exhaustive key set, no extra or missing fields
      const keys = Object.keys(response).sort();
      expect(keys).toEqual(
        [
          'capabilities',
          'chain',
          'channelCount',
          'discoveredPeerCount',
          'ilpAddress',
          'peerCount',
          'phase',
          'pricing',
          'pubkey',
          'sdk',
          'status',
          'timestamp',
          'version',
          'x402',
        ].sort()
      );
    });

    it('[P2] response has exactly the expected keys when x402 disabled (schema strictness)', () => {
      // Arrange
      const config = _createHealthConfig({ x402Enabled: false });

      // Act
      const response = createHealthResponse(config);

      // Assert -- x402 key must NOT be present
      const keys = Object.keys(response).sort();
      expect(keys).toEqual(
        [
          'capabilities',
          'chain',
          'channelCount',
          'discoveredPeerCount',
          'ilpAddress',
          'peerCount',
          'phase',
          'pricing',
          'pubkey',
          'sdk',
          'status',
          'timestamp',
          'version',
        ].sort()
      );
    });

    it('[P2] capabilities array has exact contents when x402 enabled (AC #1)', () => {
      // Arrange
      const config = _createHealthConfig({ x402Enabled: true });

      // Act
      const response = createHealthResponse(config);

      // Assert -- exactly ['relay', 'x402'], no more, no less
      expect(response.capabilities).toEqual(['relay', 'x402']);
    });

    it('[P2] capabilities array has exact contents when x402 disabled (AC #2)', () => {
      // Arrange
      const config = _createHealthConfig({ x402Enabled: false });

      // Act
      const response = createHealthResponse(config);

      // Assert -- exactly ['relay'], no more, no less
      expect(response.capabilities).toEqual(['relay']);
    });

    it('[P2] basePricePerByte of 0n converts to 0 (free relay edge case)', () => {
      // Arrange -- zero pricing is valid for free relays
      const config = _createHealthConfig({ basePricePerByte: 0n });

      // Act
      const response = createHealthResponse(config);

      // Assert -- Number(0n) === 0, not NaN or undefined
      expect(response.pricing.basePricePerByte).toBe(0);
      expect(typeof response.pricing.basePricePerByte).toBe('number');
    });

    it('[P2] basePricePerByte precision boundary (Number.MAX_SAFE_INTEGER)', () => {
      // Arrange -- test the precision boundary for bigint-to-number conversion.
      // Number.MAX_SAFE_INTEGER (2^53 - 1) is the largest integer representable
      // exactly as a JavaScript number. Values beyond this lose precision.
      const maxSafe = BigInt(Number.MAX_SAFE_INTEGER);
      const config = _createHealthConfig({ basePricePerByte: maxSafe });

      // Act
      const response = createHealthResponse(config);

      // Assert -- at the boundary, conversion is exact
      expect(response.pricing.basePricePerByte).toBe(Number.MAX_SAFE_INTEGER);
      expect(typeof response.pricing.basePricePerByte).toBe('number');
    });

    it('[P2] discoveredPeerCount and peerCount can be independently zero', () => {
      // Arrange -- peers discovered but not registered, or registered but no new discoveries
      const config1 = _createHealthConfig({
        peerCount: 0,
        discoveredPeerCount: 5,
      });
      const config2 = _createHealthConfig({
        peerCount: 3,
        discoveredPeerCount: 0,
      });

      // Act
      const response1 = createHealthResponse(config1);
      const response2 = createHealthResponse(config2);

      // Assert -- counts are independent
      expect(response1.peerCount).toBe(0);
      expect(response1.discoveredPeerCount).toBe(5);
      expect(response2.peerCount).toBe(3);
      expect(response2.discoveredPeerCount).toBe(0);
    });
  });
});

// ============================================================================
// Story 4.2: TEE Attestation Health Tests (T-4.2-06)
// These tests validate the tee field in the health response.
// Placed here because core cannot import from @crosstown/town.
// ============================================================================

describe('Story 4.2: /health TEE attestation field (T-4.2-06)', () => {
  it('[P1] includes tee field in health response when tee config is provided (T-4.2-06)', () => {
    // Arrange: simulate TEE_ENABLED=true by passing tee config
    const config = _createHealthConfig({
      x402Enabled: false,
      tee: {
        attested: true,
        enclaveType: 'aws-nitro',
        lastAttestation: Math.floor(Date.now() / 1000),
        pcr0: 'a'.repeat(96),
        state: 'valid' as const,
      },
    });
    const response = createHealthResponse(config);

    // Assert: tee field present with correct structure
    expect(response.tee).toBeDefined();
    expect(response.tee!.attested).toBe(true);
    expect(response.tee!.enclaveType).toBe('aws-nitro');
    expect(response.tee!.state).toBe('valid');
    expect(response.tee!.pcr0).toBe('a'.repeat(96));
    expect(response.tee!.lastAttestation).toBeGreaterThan(0);
  });

  it('[P1] omits tee field in health response when tee config is not provided (T-4.2-06)', () => {
    // Arrange: simulate TEE_ENABLED not set (no tee config passed)
    const config = _createHealthConfig({
      x402Enabled: false,
      // NOTE: no tee field -- enforcement guideline 12
    });
    const response = createHealthResponse(config);

    // Assert: tee field entirely absent (enforcement guideline 12)
    expect(response.tee).toBeUndefined();
    expect('tee' in response).toBe(false);
  });

  it('[P1] tee field has stale state when attestation is expired', () => {
    // Arrange: tee with stale state
    const config = _createHealthConfig({
      x402Enabled: false,
      tee: {
        attested: true,
        enclaveType: 'marlin-oyster',
        lastAttestation: Math.floor(Date.now() / 1000) - 600,
        pcr0: '0'.repeat(96),
        state: 'stale' as const,
      },
    });
    const response = createHealthResponse(config);

    // Assert: tee field reflects stale state
    expect(response.tee).toBeDefined();
    expect(response.tee!.state).toBe('stale');
    expect(response.tee!.enclaveType).toBe('marlin-oyster');
  });

  it('[P1] tee field has unattested state when no attestation published yet', () => {
    // Arrange: tee with unattested state
    const config = _createHealthConfig({
      x402Enabled: false,
      tee: {
        attested: false,
        enclaveType: 'aws-nitro',
        lastAttestation: 0,
        pcr0: '',
        state: 'unattested' as const,
      },
    });
    const response = createHealthResponse(config);

    // Assert: tee field reflects unattested state
    expect(response.tee).toBeDefined();
    expect(response.tee!.state).toBe('unattested');
    expect(response.tee!.attested).toBe(false);
  });

  // --------------------------------------------------------------------------
  // Gap-fill: Schema strictness with tee field (AC #4)
  // --------------------------------------------------------------------------
  it('[P1] response has exactly the expected keys when tee is provided (schema strictness)', () => {
    // Arrange: x402 disabled, tee enabled
    const config = _createHealthConfig({
      x402Enabled: false,
      tee: {
        attested: true,
        enclaveType: 'aws-nitro',
        lastAttestation: Math.floor(Date.now() / 1000),
        pcr0: 'a'.repeat(96),
        state: 'valid' as const,
      },
    });

    // Act
    const response = createHealthResponse(config);

    // Assert: key set includes 'tee' field
    const keys = Object.keys(response).sort();
    expect(keys).toEqual(
      [
        'capabilities',
        'chain',
        'channelCount',
        'discoveredPeerCount',
        'ilpAddress',
        'peerCount',
        'phase',
        'pricing',
        'pubkey',
        'sdk',
        'status',
        'tee',
        'timestamp',
        'version',
      ].sort()
    );
  });

  it('[P1] response has expected keys when both x402 and tee are provided', () => {
    // Arrange: both x402 and tee enabled
    const config = _createHealthConfig({
      x402Enabled: true,
      tee: {
        attested: true,
        enclaveType: 'marlin-oyster',
        lastAttestation: Math.floor(Date.now() / 1000),
        pcr0: 'f'.repeat(96),
        state: 'valid' as const,
      },
    });

    // Act
    const response = createHealthResponse(config);

    // Assert: key set includes both 'x402' and 'tee'
    const keys = Object.keys(response).sort();
    expect(keys).toEqual(
      [
        'capabilities',
        'chain',
        'channelCount',
        'discoveredPeerCount',
        'ilpAddress',
        'peerCount',
        'phase',
        'pricing',
        'pubkey',
        'sdk',
        'status',
        'tee',
        'timestamp',
        'version',
        'x402',
      ].sort()
    );
  });

  it('[P1] tee field has exact TeeHealthInfo shape', () => {
    // Arrange: tee with all fields
    const tee = {
      attested: true,
      enclaveType: 'aws-nitro',
      lastAttestation: 1710450000,
      pcr0: 'a'.repeat(96),
      state: 'valid' as const,
    };
    const config = _createHealthConfig({
      x402Enabled: false,
      tee,
    });

    // Act
    const response = createHealthResponse(config);

    // Assert: tee field has exactly the 5 TeeHealthInfo fields
    expect(response.tee).toBeDefined();
    const teeKeys = Object.keys(response.tee!).sort();
    expect(teeKeys).toEqual(
      ['attested', 'enclaveType', 'lastAttestation', 'pcr0', 'state'].sort()
    );
  });

  it('[P1] capabilities array does NOT include tee when tee is provided', () => {
    // Arrange: tee enabled -- capabilities should still be based on x402, not tee
    const config = _createHealthConfig({
      x402Enabled: false,
      tee: {
        attested: true,
        enclaveType: 'aws-nitro',
        lastAttestation: Math.floor(Date.now() / 1000),
        pcr0: 'a'.repeat(96),
        state: 'valid' as const,
      },
    });

    // Act
    const response = createHealthResponse(config);

    // Assert: tee does not appear in capabilities (it's a separate field, not a capability)
    expect(response.capabilities).toEqual(['relay']);
    expect(response.capabilities).not.toContain('tee');
  });
});
