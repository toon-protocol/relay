/**
 * Unit tests for startTown() config validation, defaults, and type exports.
 *
 * Story 2.5 ACs covered:
 * - AC #2: TownConfig validation (mnemonic/secretKey mutual exclusivity)
 * - AC #2: ResolvedTownConfig type export
 * - AC #3: Sensible defaults for config resolution
 *
 * These tests validate synchronous behavior (validation, config resolution)
 * that can be tested without genesis node infrastructure. Tests that require
 * infrastructure (starting servers, bootstrap) are in tests/e2e/.
 */

import { describe, it, expect } from 'vitest';

import { startTown, deriveAdminUrl } from './town.js';
import type { TownConfig, TownInstance, ResolvedTownConfig } from './town.js';

// ============================================================================
// Identity Validation Tests (AC #2)
// ============================================================================

describe('startTown() identity validation (AC #2)', () => {
  it('should reject config with both mnemonic and secretKey', async () => {
    // Given: a config providing BOTH identity sources
    // startTown() must throw before attempting any I/O.
    const config: TownConfig = {
      mnemonic: 'test test test test test test test test test test test junk',
      secretKey: new Uint8Array(32),
      connectorUrl: 'http://localhost:8080',
    };

    // When/Then: should throw immediately
    await expect(startTown(config)).rejects.toThrow(
      /provide either mnemonic or secretKey, not both/
    );
  });

  it('should reject config with neither mnemonic nor secretKey', async () => {
    // Given: a config with no identity source
    // TypeScript won't catch this at compile time since both are optional.
    const config = {
      connectorUrl: 'http://localhost:8080',
    } as TownConfig;

    // When/Then: should throw immediately
    await expect(startTown(config)).rejects.toThrow(
      /one of mnemonic or secretKey is required/
    );
  });
});

// ============================================================================
// Type Export Tests (AC #2)
// ============================================================================

describe('TownConfig type surface (AC #2)', () => {
  it('should accept minimal mnemonic config', () => {
    // Given: the minimum required fields
    const config: TownConfig = {
      mnemonic: 'test test test test test test test test test test test junk',
      connectorUrl: 'http://localhost:8080',
    };

    // Then: should compile and have expected structure
    expect(config.mnemonic).toBeDefined();
    expect(config.connectorUrl).toBe('http://localhost:8080');
    expect(config.relayPort).toBeUndefined();
    expect(config.blsPort).toBeUndefined();
    expect(config.basePricePerByte).toBeUndefined();
    expect(config.knownPeers).toBeUndefined();
    expect(config.chainRpcUrls).toBeUndefined();
    expect(config.tokenNetworks).toBeUndefined();
    expect(config.preferredTokens).toBeUndefined();
    expect(config.dataDir).toBeUndefined();
    expect(config.devMode).toBeUndefined();
    expect(config.ardriveEnabled).toBeUndefined();
    expect(config.relayUrls).toBeUndefined();
    expect(config.assetCode).toBeUndefined();
    expect(config.assetScale).toBeUndefined();
  });

  it('should accept secretKey config', () => {
    // Given: a config with secretKey instead of mnemonic
    const config: TownConfig = {
      secretKey: new Uint8Array(32),
      connectorUrl: 'http://localhost:8080',
    };

    expect(config.secretKey).toBeDefined();
    expect(config.mnemonic).toBeUndefined();
  });

  it('should accept full config with all optional fields', () => {
    // Given: a config with every optional field populated
    const config: TownConfig = {
      mnemonic: 'test test test test test test test test test test test junk',
      connectorUrl: 'http://localhost:8080',
      connectorAdminUrl: 'http://localhost:8081',
      relayPort: 7200,
      blsPort: 3200,
      ilpAddress: 'g.crosstown.mynode',
      btpEndpoint: 'ws://localhost:3000',
      basePricePerByte: 20n,
      knownPeers: [
        {
          pubkey: 'a'.repeat(64),
          relayUrl: 'ws://localhost:7100',
          btpEndpoint: 'ws://localhost:3000',
        },
      ],
      chainRpcUrls: { 'evm:base:31337': 'http://localhost:8545' },
      tokenNetworks: { 'evm:base:31337': '0x' + 'a'.repeat(40) },
      preferredTokens: { 'evm:base:31337': '0x' + 'b'.repeat(40) },
      dataDir: '/tmp/test-town',
      devMode: true,
      ardriveEnabled: false,
      relayUrls: ['wss://relay.example.com'],
      assetCode: 'EUR',
      assetScale: 8,
    };

    expect(config.relayPort).toBe(7200);
    expect(config.blsPort).toBe(3200);
    expect(config.basePricePerByte).toBe(20n);
    expect(config.knownPeers).toHaveLength(1);
    expect(config.devMode).toBe(true);
    expect(config.assetCode).toBe('EUR');
    expect(config.assetScale).toBe(8);
  });

  it('ResolvedTownConfig should have all fields non-optional', () => {
    // Verify the ResolvedTownConfig type has all defaults filled in
    // by constructing a valid instance (compile-time check).
    const resolved: ResolvedTownConfig = {
      relayPort: 7100,
      blsPort: 3100,
      ilpAddress: 'g.crosstown.test',
      btpEndpoint: 'ws://localhost:3000',
      connectorUrl: 'http://localhost:8080',
      connectorAdminUrl: 'http://localhost:8081',
      basePricePerByte: 10n,
      knownPeers: [],
      dataDir: './data',
      devMode: false,
      ardriveEnabled: false,
      relayUrls: [],
      assetCode: 'USD',
      assetScale: 6,
    };

    // All fields must be defined (non-optional in ResolvedTownConfig)
    expect(resolved.relayPort).toBe(7100);
    expect(resolved.blsPort).toBe(3100);
    expect(resolved.ilpAddress).toBe('g.crosstown.test');
    expect(resolved.btpEndpoint).toBe('ws://localhost:3000');
    expect(resolved.connectorUrl).toBe('http://localhost:8080');
    expect(resolved.connectorAdminUrl).toBe('http://localhost:8081');
    expect(resolved.basePricePerByte).toBe(10n);
    expect(resolved.knownPeers).toEqual([]);
    expect(resolved.dataDir).toBe('./data');
    expect(resolved.devMode).toBe(false);
    expect(resolved.ardriveEnabled).toBe(false);
    expect(resolved.relayUrls).toEqual([]);
    expect(resolved.assetCode).toBe('USD');
    expect(resolved.assetScale).toBe(6);
  });
});

// ============================================================================
// TownInstance type surface (AC #5)
// ============================================================================

describe('TownInstance type surface (AC #5)', () => {
  it('TownInstance interface should have required methods and properties', () => {
    // Verify the TownInstance type shape is correct via compile-time check.
    // We create a mock conforming to the interface.
    const mockInstance: TownInstance = {
      isRunning: () => true,
      stop: async () => {},
      subscribe: () => ({
        close: () => {},
        relayUrl: 'wss://mock.example.com',
        isActive: () => true,
      }),
      pubkey: 'a'.repeat(64),
      evmAddress: '0x' + 'b'.repeat(40),
      config: {
        relayPort: 7100,
        blsPort: 3100,
        ilpAddress: 'g.crosstown.test',
        btpEndpoint: 'ws://localhost:3000',
        connectorUrl: 'http://localhost:8080',
        connectorAdminUrl: 'http://localhost:8081',
        basePricePerByte: 10n,
        knownPeers: [],
        dataDir: './data',
        devMode: false,
        ardriveEnabled: false,
        relayUrls: [],
        assetCode: 'USD',
        assetScale: 6,
      },
      bootstrapResult: {
        peerCount: 0,
        channelCount: 0,
      },
    };

    // Verify all required members exist
    expect(typeof mockInstance.isRunning).toBe('function');
    expect(typeof mockInstance.stop).toBe('function');
    expect(typeof mockInstance.pubkey).toBe('string');
    expect(mockInstance.pubkey).toHaveLength(64);
    expect(typeof mockInstance.evmAddress).toBe('string');
    expect(mockInstance.evmAddress).toMatch(/^0x/);
    expect(mockInstance.config).toBeDefined();
    expect(mockInstance.bootstrapResult).toBeDefined();
    expect(typeof mockInstance.bootstrapResult.peerCount).toBe('number');
    expect(typeof mockInstance.bootstrapResult.channelCount).toBe('number');
  });
});

// ============================================================================
// Internal helper: deriveAdminUrl
// ============================================================================

describe('deriveAdminUrl()', () => {
  it('should increment port by 1 for standard URL', () => {
    expect(deriveAdminUrl('http://localhost:8080')).toBe(
      'http://localhost:8081'
    );
  });

  it('should handle non-standard ports', () => {
    expect(deriveAdminUrl('http://connector.example.com:9090')).toBe(
      'http://connector.example.com:9091'
    );
  });

  it('should handle HTTPS URLs with explicit non-default ports', () => {
    expect(deriveAdminUrl('https://connector.example.com:8443')).toBe(
      'https://connector.example.com:8444'
    );
  });

  it('should default to port 8080 when no port is specified', () => {
    // URL without explicit port defaults to 8080 in the function
    const result = deriveAdminUrl('http://connector.example.com');
    expect(result).toBe('http://connector.example.com:8081');
  });

  it('should strip trailing slash from result', () => {
    const result = deriveAdminUrl('http://localhost:8080/');
    expect(result).toBe('http://localhost:8081');
  });
});

// ============================================================================
// connectorAdminUrl in TownConfig
// ============================================================================

describe('connectorAdminUrl in TownConfig', () => {
  it('should be optional (defaults to derived URL from connectorUrl)', () => {
    const config: TownConfig = {
      mnemonic: 'test test test test test test test test test test test junk',
      connectorUrl: 'http://localhost:8080',
    };

    expect(config.connectorAdminUrl).toBeUndefined();
  });

  it('should use explicit connectorAdminUrl when provided', () => {
    const config: TownConfig = {
      mnemonic: 'test test test test test test test test test test test junk',
      connectorUrl: 'http://localhost:8080',
      connectorAdminUrl: 'http://custom-admin:9999',
    };

    expect(config.connectorAdminUrl).toBe('http://custom-admin:9999');
  });
});

// ============================================================================
// Module exports (AC #2)
// ============================================================================

describe('Module exports from @crosstown/town (AC #2)', () => {
  it('should export startTown as a function', async () => {
    // Dynamic import to verify the actual module exports
    const townModule = await import('./index.js');

    expect(typeof townModule.startTown).toBe('function');
  });

  it('should export handler factories alongside startTown', async () => {
    // AC #2 says existing exports must be preserved
    const townModule = await import('./index.js');

    expect(typeof townModule.createEventStorageHandler).toBe('function');
  });
});
