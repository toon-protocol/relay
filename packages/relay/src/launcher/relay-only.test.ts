/**
 * Relay-only sentinel (`chain: 'none'` / `TOON_CHAIN=none`).
 *
 * When the Townhouse network resolver has no settlement chain for a node (the
 * common case today, since TOON's on-chain contracts aren't deployed to public
 * chains), it sets the chain to the `'none'` sentinel. The town node must then
 * skip chain resolution entirely — no ethers `JsonRpcProvider` is constructed,
 * so the node connects straight to its parent connector instead of looping on
 * "JsonRpcProvider failed to detect network".
 *
 * Verified by source assertion (the package's established pattern for startTown
 * behavior — see fee-per-event-env.test.ts), avoiding a heavyweight full-boot
 * harness.
 */

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const TOWN_SOURCE = readFileSync(
  resolve(import.meta.dirname, 'town.ts'),
  'utf-8'
);

describe('relay-only sentinel (chain=none)', () => {
  it('detects the sentinel using TOON_CHAIN precedence before resolveChainConfig', () => {
    // Must read TOON_CHAIN (env wins, mirroring resolveChainConfig) and compare
    // to 'none' BEFORE calling resolveChainConfig (which throws on unknown names).
    expect(TOWN_SOURCE).toMatch(
      /process\.env\['TOON_CHAIN'\]\s*\|\|\s*config\.chain/
    );
    expect(TOWN_SOURCE).toMatch(/relayOnly\s*=\s*requestedChain === 'none'/);
  });

  it('skips resolveChainConfig when relay-only', () => {
    // The chainConfig assignment must branch on relayOnly so resolveChainConfig
    // is not called for the 'none' sentinel.
    expect(TOWN_SOURCE).toMatch(
      /relayOnly\s*\?[\s\S]*?:\s*resolveChainConfig\(config\.chain\)/
    );
  });

  it('builds no settlement maps in relay-only mode', () => {
    // effectiveChainRpcUrls / effectivePreferredTokens must be undefined when
    // relay-only, so `hasSettlement` is false and no provider is wired.
    expect(TOWN_SOURCE).toMatch(
      /effectiveChainRpcUrls\s*=[\s\S]*?relayOnly \? undefined/
    );
    expect(TOWN_SOURCE).toMatch(
      /effectivePreferredTokens\s*=[\s\S]*?relayOnly \? undefined/
    );
  });

  it('logs that it is running relay-only', () => {
    expect(TOWN_SOURCE).toContain('connector.relay_only');
  });
});
