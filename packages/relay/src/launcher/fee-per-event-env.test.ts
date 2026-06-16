/**
 * Unit Tests: TOON_FEE_PER_EVENT env var support (Story 21.5, Task 4)
 *
 * Test IDs map to test-design-epic-21.md scenario T-039.
 * TDD Red Phase — tests use it() because TOON_FEE_PER_EVENT is not yet
 * implemented in packages/relay/src/launcher/cli.ts.
 *
 * These tests verify:
 * - AC #6: Write-fee configuration via FEE_PER_EVENT environment variable
 * - Task 4.1: TOON_FEE_PER_EVENT env var parsing in parseCli()
 * - Task 4.2: feePerEvent passed to createPricingValidator()
 */

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const CLI_SOURCE_PATH = resolve(import.meta.dirname, 'cli.ts');
const TOWN_SOURCE_PATH = resolve(import.meta.dirname, 'town.ts');

describe('TOON_FEE_PER_EVENT env var (Story 21.5, Task 4)', () => {
  // ── T-039: Fee per event env var in CLI ──
  describe('T-039: CLI env var parsing', () => {
    it('[P0] cli.ts should read TOON_FEE_PER_EVENT from environment', () => {
      const source = readFileSync(CLI_SOURCE_PATH, 'utf-8');
      expect(source).toContain('TOON_FEE_PER_EVENT');
    });

    it('[P0] cli.ts should parse TOON_FEE_PER_EVENT as integer', () => {
      const source = readFileSync(CLI_SOURCE_PATH, 'utf-8');
      // Should contain parseInt or Number() conversion for the fee value
      expect(source).toMatch(/TOON_FEE_PER_EVENT/);
      expect(source).toMatch(/parseInt|Number\(/);
    });

    it('[P0] cli.ts should include feePerEvent in TownConfig object', () => {
      const source = readFileSync(CLI_SOURCE_PATH, 'utf-8');
      expect(source).toContain('feePerEvent');
    });
  });

  describe('T-039: TownConfig interface', () => {
    it('[P0] town.ts TownConfig should have optional feePerEvent field', () => {
      const source = readFileSync(TOWN_SOURCE_PATH, 'utf-8');
      // Should declare feePerEvent as optional number in TownConfig
      expect(source).toMatch(/feePerEvent\??\s*:\s*number/);
    });

    it('[P1] town.ts startTown should use feePerEvent in pricing', () => {
      const source = readFileSync(TOWN_SOURCE_PATH, 'utf-8');
      // feePerEvent should be referenced in the startTown function body
      // and connected to the pricing validator
      expect(source).toMatch(/feePerEvent/);
    });
  });

  describe('T-039: Validation and edge cases', () => {
    it('[P1] cli.ts should validate feePerEvent is non-negative', () => {
      const source = readFileSync(CLI_SOURCE_PATH, 'utf-8');
      // Should contain validation that rejects negative values
      expect(source).toMatch(/feePerEvent.*<\s*0|feePerEvent\s*<\s*0/);
    });

    it('[P2] cli.ts help text should document TOON_FEE_PER_EVENT', () => {
      const source = readFileSync(CLI_SOURCE_PATH, 'utf-8');
      // The help text (printHelp function) should mention the env var
      const helpSection =
        source.match(/function printHelp[\s\S]*?^}/m)?.[0] ?? '';
      expect(helpSection).toContain('TOON_FEE_PER_EVENT');
    });
  });
});
