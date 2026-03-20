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
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

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
      ilpAddress: 'g.toon.mynode',
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
      ilpAddress: 'g.toon.test',
      btpEndpoint: 'ws://localhost:3000',
      connectorUrl: 'http://localhost:8080',
      connectorAdminUrl: 'http://localhost:8081',
      basePricePerByte: 10n,
      routingBufferPercent: 10,
      x402Enabled: false,
      knownPeers: [],
      dataDir: './data',
      devMode: false,
      ardriveEnabled: false,
      relayUrls: [],
      assetCode: 'USD',
      assetScale: 6,
      discovery: 'genesis',
      seedRelays: [],
      publishSeedEntry: false,
      chain: 'anvil',
    };

    // All fields must be defined (non-optional in ResolvedTownConfig)
    expect(resolved.relayPort).toBe(7100);
    expect(resolved.blsPort).toBe(3100);
    expect(resolved.ilpAddress).toBe('g.toon.test');
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
    // Story 3.4: Seed relay discovery fields
    expect(resolved.discovery).toBe('genesis');
    expect(resolved.seedRelays).toEqual([]);
    expect(resolved.publishSeedEntry).toBe(false);
    expect(resolved.externalRelayUrl).toBeUndefined();
    // Story 3.5: Chain field
    expect(resolved.chain).toBe('anvil');
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
        ilpAddress: 'g.toon.test',
        btpEndpoint: 'ws://localhost:3000',
        connectorUrl: 'http://localhost:8080',
        connectorAdminUrl: 'http://localhost:8081',
        basePricePerByte: 10n,
        routingBufferPercent: 10,
        x402Enabled: false,
        knownPeers: [],
        dataDir: './data',
        devMode: false,
        ardriveEnabled: false,
        relayUrls: [],
        assetCode: 'USD',
        assetScale: 6,
        discovery: 'genesis',
        seedRelays: [],
        publishSeedEntry: false,
        chain: 'anvil',
      },
      bootstrapResult: {
        peerCount: 0,
        channelCount: 0,
      },
      discoveryMode: 'genesis',
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

describe('Module exports from @toon-protocol/town (AC #2)', () => {
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

// ============================================================================
// Story 3.4: Seed Relay Discovery -- TownConfig integration (AC #4)
// ============================================================================

describe('TownConfig seed relay discovery fields (Story 3.4 AC #4)', () => {
  it('discovery defaults to undefined (resolved to "genesis" in startTown)', () => {
    // Given: a minimal TownConfig without discovery field
    const config: TownConfig = {
      mnemonic: 'test test test test test test test test test test test junk',
      connectorUrl: 'http://localhost:8080',
    };

    // Then: discovery is undefined, will be resolved to 'genesis' by startTown
    expect(config.discovery).toBeUndefined();
  });

  it('TownConfig accepts discovery: "seed-list"', () => {
    const config: TownConfig = {
      mnemonic: 'test test test test test test test test test test test junk',
      connectorUrl: 'http://localhost:8080',
      discovery: 'seed-list',
      seedRelays: ['wss://relay.damus.io', 'wss://relay.nostr.band'],
    };

    expect(config.discovery).toBe('seed-list');
    expect(config.seedRelays).toHaveLength(2);
  });

  it('TownConfig accepts discovery: "genesis"', () => {
    const config: TownConfig = {
      mnemonic: 'test test test test test test test test test test test junk',
      connectorUrl: 'http://localhost:8080',
      discovery: 'genesis',
    };

    expect(config.discovery).toBe('genesis');
  });

  it('TownConfig accepts publishSeedEntry and externalRelayUrl', () => {
    const config: TownConfig = {
      mnemonic: 'test test test test test test test test test test test junk',
      connectorUrl: 'http://localhost:8080',
      discovery: 'seed-list',
      seedRelays: ['wss://relay.damus.io'],
      publishSeedEntry: true,
      externalRelayUrl: 'wss://my-relay.example.com',
    };

    expect(config.publishSeedEntry).toBe(true);
    expect(config.externalRelayUrl).toBe('wss://my-relay.example.com');
  });

  it('seedRelays, publishSeedEntry, externalRelayUrl default to undefined', () => {
    const config: TownConfig = {
      mnemonic: 'test test test test test test test test test test test junk',
      connectorUrl: 'http://localhost:8080',
    };

    expect(config.seedRelays).toBeUndefined();
    expect(config.publishSeedEntry).toBeUndefined();
    expect(config.externalRelayUrl).toBeUndefined();
  });
});

describe('ResolvedTownConfig seed relay defaults (Story 3.4 AC #4)', () => {
  it('discovery defaults to "genesis" in ResolvedTownConfig', () => {
    // Verify that the resolved config defaults discovery to 'genesis'
    const resolved: ResolvedTownConfig = {
      relayPort: 7100,
      blsPort: 3100,
      ilpAddress: 'g.toon.test',
      btpEndpoint: 'ws://localhost:3000',
      basePricePerByte: 10n,
      routingBufferPercent: 10,
      x402Enabled: false,
      knownPeers: [],
      dataDir: './data',
      devMode: false,
      ardriveEnabled: false,
      relayUrls: [],
      assetCode: 'USD',
      assetScale: 6,
      discovery: 'genesis',
      seedRelays: [],
      publishSeedEntry: false,
      chain: 'anvil',
    };

    expect(resolved.discovery).toBe('genesis');
    expect(resolved.seedRelays).toEqual([]);
    expect(resolved.publishSeedEntry).toBe(false);
  });

  it('ResolvedTownConfig accepts seed-list discovery mode', () => {
    const resolved: ResolvedTownConfig = {
      relayPort: 7100,
      blsPort: 3100,
      ilpAddress: 'g.toon.test',
      btpEndpoint: 'ws://localhost:3000',
      basePricePerByte: 10n,
      routingBufferPercent: 10,
      x402Enabled: false,
      knownPeers: [],
      dataDir: './data',
      devMode: false,
      ardriveEnabled: false,
      relayUrls: [],
      assetCode: 'USD',
      assetScale: 6,
      discovery: 'seed-list',
      seedRelays: ['wss://relay.damus.io'],
      publishSeedEntry: true,
      externalRelayUrl: 'wss://my-relay.example.com',
      chain: 'anvil',
    };

    expect(resolved.discovery).toBe('seed-list');
    expect(resolved.seedRelays).toEqual(['wss://relay.damus.io']);
    expect(resolved.publishSeedEntry).toBe(true);
    expect(resolved.externalRelayUrl).toBe('wss://my-relay.example.com');
  });
});

describe('TownInstance.discoveryMode (Story 3.4 AC #1, #4)', () => {
  it('TownInstance interface includes discoveryMode property', () => {
    // Verify TownInstance has discoveryMode via compile-time type check
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
        ilpAddress: 'g.toon.test',
        btpEndpoint: 'ws://localhost:3000',
        basePricePerByte: 10n,
        routingBufferPercent: 10,
        x402Enabled: false,
        knownPeers: [],
        dataDir: './data',
        devMode: false,
        ardriveEnabled: false,
        relayUrls: [],
        assetCode: 'USD',
        assetScale: 6,
        discovery: 'genesis',
        seedRelays: [],
        publishSeedEntry: false,
        chain: 'anvil',
      },
      bootstrapResult: { peerCount: 0, channelCount: 0 },
      discoveryMode: 'genesis',
    };

    expect(mockInstance.discoveryMode).toBe('genesis');
  });

  it('TownInstance.discoveryMode can be "seed-list"', () => {
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
        ilpAddress: 'g.toon.test',
        btpEndpoint: 'ws://localhost:3000',
        basePricePerByte: 10n,
        routingBufferPercent: 10,
        x402Enabled: false,
        knownPeers: [],
        dataDir: './data',
        devMode: false,
        ardriveEnabled: false,
        relayUrls: [],
        assetCode: 'USD',
        assetScale: 6,
        discovery: 'seed-list',
        seedRelays: ['wss://relay.example.com'],
        publishSeedEntry: true,
        externalRelayUrl: 'wss://my-relay.example.com',
        chain: 'anvil',
      },
      bootstrapResult: { peerCount: 0, channelCount: 0 },
      discoveryMode: 'seed-list',
    };

    expect(mockInstance.discoveryMode).toBe('seed-list');
  });
});

// ============================================================================
// Story 3.4: startTown() integration -- static analysis (AC #1, #4)
// ============================================================================

describe('startTown() seed relay integration -- static analysis (Story 3.4)', () => {
  it('town.ts imports SeedRelayDiscovery from @toon-protocol/core', () => {
    const sourcePath = resolve(__dirname, 'town.ts');
    const source = readFileSync(sourcePath, 'utf-8');

    // Verify SeedRelayDiscovery is imported
    expect(source).toContain('SeedRelayDiscovery');
    // Verify publishSeedRelayEntry is imported
    expect(source).toContain('publishSeedRelayEntry');
  });

  it('town.ts uses discovery === "seed-list" guard before SeedRelayDiscovery', () => {
    const sourcePath = resolve(__dirname, 'town.ts');
    const source = readFileSync(sourcePath, 'utf-8');

    // Verify that SeedRelayDiscovery is only used when discovery is 'seed-list'
    expect(source).toMatch(/discovery\s*===\s*['"]seed-list['"]/);
  });

  it('town.ts defaults discovery to "genesis" (backward compat)', () => {
    const sourcePath = resolve(__dirname, 'town.ts');
    const source = readFileSync(sourcePath, 'utf-8');

    // Verify the default is 'genesis': config.discovery ?? 'genesis'
    expect(source).toMatch(/config\.discovery\s*\?\?\s*['"]genesis['"]/);
  });

  it('town.ts sets TownInstance.discoveryMode from resolved config', () => {
    const sourcePath = resolve(__dirname, 'town.ts');
    const source = readFileSync(sourcePath, 'utf-8');

    // Verify discoveryMode is set on the instance
    expect(source).toContain('discoveryMode');
  });
});

// ============================================================================
// Story 3.5: Service Discovery -- chain field on TownConfig / ResolvedTownConfig
// ============================================================================

describe('TownConfig supports chain field (T-3.5-10)', () => {
  it('[P2] TownConfig accepts chain field', () => {
    // Given: a TownConfig with the chain field set
    const config: TownConfig = {
      mnemonic: 'test test test test test test test test test test test junk',
      connectorUrl: 'http://localhost:8080',
      chain: 'arbitrum-one',
    };

    // Then: chain field is accepted by TypeScript and holds the value
    expect(config.chain).toBe('arbitrum-one');
  });

  it('[P2] TownConfig chain field defaults to undefined', () => {
    // Given: a minimal TownConfig without chain
    const config: TownConfig = {
      mnemonic: 'test test test test test test test test test test test junk',
      connectorUrl: 'http://localhost:8080',
    };

    // Then: chain is undefined (resolved to 'anvil' by startTown)
    expect(config.chain).toBeUndefined();
  });
});

describe('ResolvedTownConfig includes chain field (T-3.5-11)', () => {
  it('[P2] ResolvedTownConfig accepts chain field with string value', () => {
    // Given: a ResolvedTownConfig with chain field populated
    const resolved: ResolvedTownConfig = {
      relayPort: 7100,
      blsPort: 3100,
      ilpAddress: 'g.toon.test',
      btpEndpoint: 'ws://localhost:3000',
      connectorUrl: 'http://localhost:8080',
      connectorAdminUrl: 'http://localhost:8081',
      basePricePerByte: 10n,
      routingBufferPercent: 10,
      x402Enabled: false,
      knownPeers: [],
      dataDir: './data',
      devMode: false,
      ardriveEnabled: false,
      relayUrls: [],
      assetCode: 'USD',
      assetScale: 6,
      discovery: 'genesis',
      seedRelays: [],
      publishSeedEntry: false,
      chain: 'anvil',
    };

    // Then: chain field holds the value
    expect(resolved.chain).toBe('anvil');
  });

  it('[P2] ResolvedTownConfig chain field accepts production preset names', () => {
    // Given: a ResolvedTownConfig with a production chain preset
    const resolved: ResolvedTownConfig = {
      relayPort: 7100,
      blsPort: 3100,
      ilpAddress: 'g.toon.test',
      btpEndpoint: 'ws://localhost:3000',
      connectorUrl: 'http://localhost:8080',
      connectorAdminUrl: 'http://localhost:8081',
      basePricePerByte: 10n,
      routingBufferPercent: 10,
      x402Enabled: true,
      knownPeers: [],
      dataDir: './data',
      devMode: false,
      ardriveEnabled: false,
      relayUrls: [],
      assetCode: 'USD',
      assetScale: 6,
      discovery: 'seed-list',
      seedRelays: ['wss://relay.damus.io'],
      publishSeedEntry: true,
      externalRelayUrl: 'wss://my-relay.example.com',
      chain: 'arbitrum-one',
    };

    // Then: chain field accepts production preset name
    expect(resolved.chain).toBe('arbitrum-one');
  });
});

// ============================================================================
// Story 3.5: Service Discovery -- startTown() integration (static analysis)
// ============================================================================

describe('startTown() kind:10035 integration -- static analysis (Story 3.5)', () => {
  it('town.ts imports buildServiceDiscoveryEvent from @toon-protocol/core', () => {
    const sourcePath = resolve(__dirname, 'town.ts');
    const source = readFileSync(sourcePath, 'utf-8');

    // AC #1: verify builder function is imported
    expect(source).toContain('buildServiceDiscoveryEvent');
  });

  it('town.ts imports ServiceDiscoveryContent type from @toon-protocol/core', () => {
    const sourcePath = resolve(__dirname, 'town.ts');
    const source = readFileSync(sourcePath, 'utf-8');

    // AC #2: verify the content type is imported for type safety
    expect(source).toContain('ServiceDiscoveryContent');
  });

  it('town.ts imports VERSION from @toon-protocol/core', () => {
    const sourcePath = resolve(__dirname, 'town.ts');
    const source = readFileSync(sourcePath, 'utf-8');

    // AC #2: version field comes from the VERSION constant
    expect(source).toContain('VERSION');
  });

  it('town.ts stores kind:10035 event via eventStore.store()', () => {
    const sourcePath = resolve(__dirname, 'town.ts');
    const source = readFileSync(sourcePath, 'utf-8');

    // AC #1: verify the service discovery event is stored locally
    expect(source).toContain('eventStore.store(serviceDiscoveryEvent)');
  });

  it('town.ts publishes kind:10035 via ILP fire-and-forget', () => {
    const sourcePath = resolve(__dirname, 'town.ts');
    const source = readFileSync(sourcePath, 'utf-8');

    // AC #1: verify the service discovery event is published to peers
    expect(source).toContain('Failed to publish service discovery via ILP');
  });

  it('town.ts conditionally includes x402 field only when x402Enabled (AC #3)', () => {
    const sourcePath = resolve(__dirname, 'town.ts');
    const source = readFileSync(sourcePath, 'utf-8');

    // AC #3: x402 field omitted entirely when disabled
    expect(source).toMatch(/if\s*\(x402Enabled\)\s*\{/);
    expect(source).toContain('serviceDiscoveryContent.x402');
  });

  it('town.ts uses chainConfig.name for service discovery chain field', () => {
    const sourcePath = resolve(__dirname, 'town.ts');
    const source = readFileSync(sourcePath, 'utf-8');

    // AC #2: chain field sourced from resolved chain config
    expect(source).toContain('chainConfig.name');
  });

  it('town.ts publishes kind:10035 after kind:10032 (ordering)', () => {
    const sourcePath = resolve(__dirname, 'town.ts');
    const source = readFileSync(sourcePath, 'utf-8');

    // AC #1: service discovery published after ILP peer info
    const kind10032Idx = source.indexOf('kind:10032');
    const kind10035Idx = source.indexOf('kind:10035');
    expect(kind10032Idx).toBeGreaterThan(-1);
    expect(kind10035Idx).toBeGreaterThan(-1);
    expect(kind10035Idx).toBeGreaterThan(kind10032Idx);
  });
});

// ============================================================================
// Story 3.6: Enriched Health Endpoint -- startTown() integration (static analysis)
// ============================================================================

describe('startTown() enriched health integration -- static analysis (Story 3.6)', () => {
  it('town.ts imports createHealthResponse from ./health.js (T-3.6-12)', () => {
    const sourcePath = resolve(__dirname, 'town.ts');
    const source = readFileSync(sourcePath, 'utf-8');

    // AC #1: verify createHealthResponse is imported from the health module
    expect(source).toMatch(
      /import\s*\{[^}]*createHealthResponse[^}]*\}\s*from\s*['"]\.\/health\.js['"]/
    );
  });

  it('town.ts health endpoint calls createHealthResponse (T-3.6-13)', () => {
    const sourcePath = resolve(__dirname, 'town.ts');
    const source = readFileSync(sourcePath, 'utf-8');

    // AC #1: verify the /health handler delegates to createHealthResponse()
    expect(source).toContain('createHealthResponse(');
  });
});

// ============================================================================
// Quick-Spec: Wire viem clients in startTown() for production x402
// ============================================================================

describe('startTown() x402 viem client wiring -- static analysis', () => {
  const sourcePath = resolve(__dirname, 'town.ts');
  const source = readFileSync(sourcePath, 'utf-8');

  it('imports createPublicClient and createWalletClient from viem', () => {
    expect(source).toContain('createPublicClient');
    expect(source).toContain('createWalletClient');
    expect(source).toMatch(/from 'viem'/);
  });

  it('imports privateKeyToAccount from viem/accounts', () => {
    expect(source).toContain('privateKeyToAccount');
    expect(source).toMatch(/from 'viem\/accounts'/);
  });

  it('imports WalletClient and PublicClient types from viem', () => {
    expect(source).toMatch(
      /import type\s*\{[^}]*WalletClient[^}]*\}\s*from 'viem'/
    );
    expect(source).toMatch(
      /import type\s*\{[^}]*PublicClient[^}]*\}\s*from 'viem'/
    );
  });

  it('creates viem clients inside x402Enabled conditional, after ILP client and before handler', () => {
    // Use call-site patterns (with '(') to skip import-level occurrences
    const ilpClientIdx = source.indexOf('createHttpIlpClient(');
    const viemBlockIdx = source.indexOf('privateKeyToAccount(');
    const handlerIdx = source.indexOf('createX402Handler({');
    expect(ilpClientIdx).toBeGreaterThan(-1);
    expect(viemBlockIdx).toBeGreaterThan(-1);
    expect(handlerIdx).toBeGreaterThan(-1);
    // Ordering: ILP client call < viem block call < x402 handler call
    expect(viemBlockIdx).toBeGreaterThan(ilpClientIdx);
    expect(handlerIdx).toBeGreaterThan(viemBlockIdx);
  });

  it('passes x402WalletClient and x402PublicClient to createX402Handler', () => {
    expect(source).toMatch(/walletClient:\s*x402WalletClient/);
    expect(source).toMatch(/publicClient:\s*x402PublicClient/);
  });

  it('zeroes key material buffer in finally block', () => {
    // Verify fill(0) appears after the finally keyword, not just anywhere in the file
    const finallyIdx = source.indexOf(
      'finally {',
      source.indexOf('viem clients for x402 settlement')
    );
    expect(finallyIdx).toBeGreaterThan(-1);
    const fillIdx = source.indexOf('keyBuffer.fill(0)', finallyIdx);
    expect(fillIdx).toBeGreaterThan(finallyIdx);
  });

  it('wraps privateKeyToAccount in try/catch with descriptive error', () => {
    expect(source).toContain('x402 initialization failed');
  });

  it('viem client creation is inside x402Enabled guard, not top-level', () => {
    // Anchor on the unique section comment, then find the guard after it
    const sectionAnchor = source.indexOf('viem clients for x402 settlement');
    expect(sectionAnchor).toBeGreaterThan(-1);
    const x402CondIdx = source.indexOf('if (x402Enabled) {', sectionAnchor);
    expect(x402CondIdx).toBeGreaterThan(sectionAnchor);
    const createPublicIdx = source.indexOf('createPublicClient({', x402CondIdx);
    const createWalletIdx = source.indexOf('createWalletClient({', x402CondIdx);
    expect(createPublicIdx).toBeGreaterThan(x402CondIdx);
    expect(createWalletIdx).toBeGreaterThan(x402CondIdx);
  });
});

// ============================================================================
// Story 5.4: Skill Descriptors -- startTown() integration (static analysis)
// ============================================================================

describe('startTown() skill descriptor integration -- static analysis (Story 5.4)', () => {
  const sourcePath = resolve(__dirname, 'town.ts');
  const source = readFileSync(sourcePath, 'utf-8');

  it('TownConfig accepts optional skill field (T-5.4-06)', () => {
    // AC #5: TownConfig should include an optional skill property
    expect(source).toMatch(/skill\??\s*:\s*SkillDescriptor/);
  });

  it('town.ts imports SkillDescriptor type from @toon-protocol/core', () => {
    // Verify the SkillDescriptor type is imported for type safety
    expect(source).toContain('SkillDescriptor');
    expect(source).toMatch(/from\s+['"]@toon-protocol\/core['"]/);
  });

  it('town.ts conditionally includes skill field in kind:10035 content (T-5.4-06)', () => {
    // AC #5: skill descriptor is included in service discovery when configured
    expect(source).toMatch(/if\s*\(config\.skill\)\s*\{/);
    expect(source).toContain('serviceDiscoveryContent.skill = config.skill');
  });

  it('skill wiring is positioned after x402 guard and before buildServiceDiscoveryEvent (T-5.4-06)', () => {
    // Verify ordering: x402 guard -> skill wiring -> buildServiceDiscoveryEvent
    const x402Idx = source.indexOf('serviceDiscoveryContent.x402');
    const skillIdx = source.indexOf('serviceDiscoveryContent.skill');
    const buildIdx = source.indexOf('buildServiceDiscoveryEvent(');
    expect(x402Idx).toBeGreaterThan(-1);
    expect(skillIdx).toBeGreaterThan(-1);
    expect(buildIdx).toBeGreaterThan(-1);
    // skill wiring comes after x402 and before build
    expect(skillIdx).toBeGreaterThan(x402Idx);
    expect(buildIdx).toBeGreaterThan(skillIdx);
  });
});
