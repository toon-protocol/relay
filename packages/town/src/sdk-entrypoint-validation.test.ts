/**
 * Static analysis tests for Story 2.3: E2E Test Validation
 *
 * These tests verify structural properties of the SDK-based Docker entrypoint
 * (docker/src/entrypoint-town.ts) without requiring a running genesis node.
 * They run as part of the normal `pnpm test` suite.
 *
 * Test IDs from ATDD checklist:
 * - T-2.3-07 [P1]: SDK relay entrypoint < 100 lines of handler code (AC #2)
 *
 * Additional coverage:
 * - Entrypoint imports handlers from @crosstown/town (not SDK stubs)
 * - Entrypoint includes `sdk: true` in health response
 * - Docker package.json includes SDK and Town dependencies
 */

import { describe, it, expect } from 'vitest';
import { readFileSync, existsSync } from 'fs';
import { resolve } from 'path';

// ============================================================================
// Helpers
// ============================================================================

/**
 * Resolve a path relative to the repository root.
 * This test file lives at packages/town/src/sdk-entrypoint-validation.test.ts,
 * so the repo root is three directories up.
 */
function repoRoot(): string {
  return resolve(__dirname, '..', '..', '..');
}

// ============================================================================
// Tests
// ============================================================================

describe('Story 2.3: SDK-based entrypoint validation (static)', () => {
  // --------------------------------------------------------------------------
  // T-2.3-07 [P1]: SDK relay entrypoint < 100 lines of handler code (AC #2)
  // --------------------------------------------------------------------------

  it('SDK relay entrypoint handler logic should be < 100 lines (AC #2)', () => {
    // AC #2: handler registrations are < 100 lines of handler logic
    // (non-blank, non-comment, non-import), reflecting the SDK's abstraction
    // value vs the ~300+ lines of handle-packet logic in the original.
    //
    // This counts lines within the createPipelineHandler() function body,
    // which contains the SDK pipeline: handler registry wiring, verification
    // pipeline, pricing validator, and the packet handler callback.

    const entrypointPath = resolve(
      repoRoot(),
      'docker',
      'src',
      'entrypoint-town.ts'
    );

    expect(
      existsSync(entrypointPath),
      'docker/src/entrypoint-town.ts must exist (SDK-based Docker entrypoint)'
    ).toBe(true);

    const source = readFileSync(entrypointPath, 'utf-8');
    const lines = source.split('\n');

    // Extract lines within the createPipelineHandler function body.
    // This function contains the SDK pipeline: handler registry wiring,
    // verification pipeline, pricing validator, and the packet handler callback.
    let inHandlerSection = false;
    let foundFirstBrace = false;
    let braceDepth = 0;
    const handlerLines: string[] = [];

    for (const line of lines) {
      const trimmed = line.trim();

      // Start tracking at createPipelineHandler
      if (trimmed.startsWith('function createPipelineHandler(')) {
        inHandlerSection = true;
      }

      if (inHandlerSection) {
        // Count braces to find the end of the function
        for (const ch of trimmed) {
          if (ch === '{') {
            braceDepth++;
            foundFirstBrace = true;
          }
          if (ch === '}') braceDepth--;
        }

        // Apply the standard line filter: skip blanks, comments, imports,
        // type exports, and lone closing braces
        if (trimmed === '') {
          /* skip blank lines */
        } else if (trimmed.startsWith('//')) {
          /* skip single-line comments */
        } else if (trimmed.startsWith('/*') || trimmed.startsWith('*')) {
          /* skip multi-line comment lines */
        } else if (trimmed.startsWith('import ')) {
          /* skip imports */
        } else if (
          trimmed.startsWith('export type') ||
          trimmed.startsWith('export interface')
        ) {
          /* skip type exports */
        } else if (trimmed === '}' || trimmed === '};' || trimmed === '},') {
          /* skip lone closing braces */
        } else {
          handlerLines.push(trimmed);
        }

        // End of createPipelineHandler function (after the first { is found)
        if (foundFirstBrace && braceDepth === 0) {
          inHandlerSection = false;
        }
      }
    }

    // Also count the pipeline constant outside the function
    const pipelineConstLines = lines.filter((line) => {
      const trimmed = line.trim();
      return trimmed.startsWith('const MAX_PAYLOAD_BASE64_LENGTH');
    });

    const totalHandlerLines = handlerLines.length + pipelineConstLines.length;

    // The SDK-based relay entrypoint should be < 100 lines of handler code
    expect(
      totalHandlerLines,
      `Handler logic is ${totalHandlerLines} lines (must be < 100). ` +
        `This validates AC #2: SDK entrypoint has < 100 lines of handler logic.`
    ).toBeLessThan(100);
  });

  it('SDK relay entrypoint handler logic should be significantly smaller than old entrypoint', () => {
    // The SDK-based handler code should be less than 50% of the old
    // entrypoint's total non-blank, non-comment, non-import lines.

    const entrypointPath = resolve(
      repoRoot(),
      'docker',
      'src',
      'entrypoint-town.ts'
    );
    const oldEntrypointPath = resolve(
      repoRoot(),
      'docker',
      'src',
      'entrypoint.ts'
    );

    expect(existsSync(entrypointPath), 'entrypoint-town.ts must exist').toBe(
      true
    );
    expect(existsSync(oldEntrypointPath), 'entrypoint.ts must exist').toBe(
      true
    );

    // Count handler lines in new entrypoint (same logic as above)
    const source = readFileSync(entrypointPath, 'utf-8');
    const lines = source.split('\n');
    let inHandlerSection = false;
    let foundFirstBrace = false;
    let braceDepth = 0;
    const handlerLines: string[] = [];

    for (const line of lines) {
      const trimmed = line.trim();
      if (trimmed.startsWith('function createPipelineHandler(')) {
        inHandlerSection = true;
      }
      if (inHandlerSection) {
        for (const ch of trimmed) {
          if (ch === '{') {
            braceDepth++;
            foundFirstBrace = true;
          }
          if (ch === '}') braceDepth--;
        }
        if (trimmed === '') {
          /* skip */
        } else if (trimmed.startsWith('//')) {
          /* skip */
        } else if (trimmed.startsWith('/*') || trimmed.startsWith('*')) {
          /* skip */
        } else if (trimmed.startsWith('import ')) {
          /* skip */
        } else if (trimmed === '}' || trimmed === '};' || trimmed === '},') {
          /* skip */
        } else {
          handlerLines.push(trimmed);
        }
        if (foundFirstBrace && braceDepth === 0) {
          inHandlerSection = false;
        }
      }
    }
    const pipelineConstLines = lines.filter((l) =>
      l.trim().startsWith('const MAX_PAYLOAD_BASE64_LENGTH')
    );
    const totalHandlerLines = handlerLines.length + pipelineConstLines.length;

    // Count all non-blank, non-comment, non-import lines in the old entrypoint
    const oldSource = readFileSync(oldEntrypointPath, 'utf-8');
    const oldLines = oldSource.split('\n').filter((line) => {
      const trimmed = line.trim();
      if (trimmed === '') return false;
      if (trimmed.startsWith('//')) return false;
      if (trimmed.startsWith('/*') || trimmed.startsWith('*')) return false;
      if (trimmed.startsWith('import ')) return false;
      return true;
    });

    // SDK handler code should be significantly smaller than old entrypoint
    expect(
      totalHandlerLines,
      `Handler logic (${totalHandlerLines} lines) must be < 50% of old entrypoint (${oldLines.length} lines)`
    ).toBeLessThan(oldLines.length * 0.5);
  });

  // --------------------------------------------------------------------------
  // Import validation: handlers from @crosstown/town, not @crosstown/sdk
  // --------------------------------------------------------------------------

  it('SDK relay entrypoint should import handlers from @crosstown/town (not SDK stubs)', () => {
    // Critical rule: handlers must be imported from @crosstown/town (real
    // implementations), NOT from @crosstown/sdk (which exports throwing stubs).
    const entrypointPath = resolve(
      repoRoot(),
      'docker',
      'src',
      'entrypoint-town.ts'
    );
    const source = readFileSync(entrypointPath, 'utf-8');

    // Must import createEventStorageHandler from @crosstown/town
    expect(
      source,
      'Must import createEventStorageHandler from @crosstown/town'
    ).toMatch(
      /import\s+\{[^}]*createEventStorageHandler[^}]*\}\s+from\s+['"]@crosstown\/town['"]/
    );

    // Must import createSpspHandshakeHandler from @crosstown/town
    expect(
      source,
      'Must import createSpspHandshakeHandler from @crosstown/town'
    ).toMatch(
      /import\s+\{[^}]*createSpspHandshakeHandler[^}]*\}\s+from\s+['"]@crosstown\/town['"]/
    );

    // Must NOT import handler implementations from @crosstown/sdk
    const sdkHandlerImportPattern =
      /import\s+\{[^}]*(?:createEventStorageHandler|createSpspHandshakeHandler)[^}]*\}\s+from\s+['"]@crosstown\/sdk['"]/;
    expect(
      sdkHandlerImportPattern.test(source),
      'Must NOT import handler implementations from @crosstown/sdk (they are throwing stubs)'
    ).toBe(false);
  });

  // --------------------------------------------------------------------------
  // Health endpoint: `sdk: true` field
  // --------------------------------------------------------------------------

  it('SDK relay entrypoint should include sdk:true in health response', () => {
    // The SDK-based relay health endpoint must include `sdk: true` so E2E
    // tests can detect SDK mode vs the old entrypoint.
    const entrypointPath = resolve(
      repoRoot(),
      'docker',
      'src',
      'entrypoint-town.ts'
    );
    const source = readFileSync(entrypointPath, 'utf-8');

    // The health endpoint JSON must include sdk: true
    expect(source, 'Health endpoint must include sdk: true').toMatch(
      /sdk:\s*true/
    );
  });

  // --------------------------------------------------------------------------
  // Docker package.json: SDK and Town dependencies
  // --------------------------------------------------------------------------

  it('docker/package.json should include @crosstown/sdk and @crosstown/town dependencies', () => {
    // Docker package must depend on both SDK (pipeline components) and
    // Town (handler implementations) workspace packages.
    const dockerPkgPath = resolve(repoRoot(), 'docker', 'package.json');

    expect(existsSync(dockerPkgPath), 'docker/package.json must exist').toBe(
      true
    );

    const content = readFileSync(dockerPkgPath, 'utf-8');
    const pkg = JSON.parse(content) as Record<string, unknown>;
    const deps = pkg['dependencies'] as Record<string, string> | undefined;

    expect(deps, 'docker/package.json must have dependencies').toBeDefined();
    // deps is validated as defined above; use a fallback for type safety
    const safeDeps = deps ?? {};
    expect(
      safeDeps['@crosstown/sdk'],
      'docker/package.json must depend on @crosstown/sdk'
    ).toBeDefined();
    expect(
      safeDeps['@crosstown/town'],
      'docker/package.json must depend on @crosstown/town'
    ).toBeDefined();
  });

  // --------------------------------------------------------------------------
  // Dockerfile: SDK-based entrypoint CMD
  // --------------------------------------------------------------------------

  it('Dockerfile CMD should reference entrypoint-town.js', () => {
    // The Docker image must use the SDK-based entrypoint, not the old one.
    const dockerfilePath = resolve(repoRoot(), 'docker', 'Dockerfile');

    expect(existsSync(dockerfilePath), 'docker/Dockerfile must exist').toBe(
      true
    );

    const content = readFileSync(dockerfilePath, 'utf-8');

    // CMD must reference entrypoint-town.js
    expect(content, 'Dockerfile CMD must reference entrypoint-town.js').toMatch(
      /CMD.*entrypoint-town\.js/
    );
  });

  // --------------------------------------------------------------------------
  // SDK pipeline composition: required components
  // --------------------------------------------------------------------------

  it('SDK relay entrypoint should compose the full pipeline (verify, price, dispatch)', () => {
    // The entrypoint must use all SDK pipeline components:
    // createVerificationPipeline, createPricingValidator, HandlerRegistry
    const entrypointPath = resolve(
      repoRoot(),
      'docker',
      'src',
      'entrypoint-town.ts'
    );
    const source = readFileSync(entrypointPath, 'utf-8');

    // Must import and use SDK pipeline components
    expect(source).toMatch(/createVerificationPipeline/);
    expect(source).toMatch(/createPricingValidator/);
    expect(source).toMatch(/HandlerRegistry/);
    expect(source).toMatch(/createHandlerContext/);

    // Must use shallowParseToon for TOON parsing
    expect(source).toMatch(/shallowParseToon/);

    // Must have size check for payloads
    expect(source).toMatch(/MAX_PAYLOAD_BASE64_LENGTH/);
  });
});
