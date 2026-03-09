/**
 * Unit tests for @crosstown/town package structure and publishability.
 *
 * Story 2.5 ACs covered:
 * - AC #1: package.json structure (deps, bin, type:module, TypeScript strict)
 * - AC #6: npm publishability (ESM exports, publishConfig, files field)
 *
 * These are static filesystem checks that run without infrastructure.
 */

import { describe, it, expect } from 'vitest';
import { readFileSync, existsSync } from 'fs';
import { resolve } from 'path';

// ============================================================================
// Helpers
// ============================================================================

function repoRoot(): string {
  return resolve(__dirname, '..', '..', '..');
}

function townPackagePath(): string {
  return resolve(repoRoot(), 'packages', 'town', 'package.json');
}

function townTsconfigPath(): string {
  return resolve(repoRoot(), 'packages', 'town', 'tsconfig.json');
}

function readJson(filePath: string): Record<string, unknown> {
  return JSON.parse(readFileSync(filePath, 'utf-8')) as Record<string, unknown>;
}

// ============================================================================
// AC #1: package.json structure
// ============================================================================

describe('AC #1: package.json dependencies and structure', () => {
  it('should have @crosstown/sdk as a runtime dependency', () => {
    const pkg = readJson(townPackagePath());
    const deps = pkg['dependencies'] as Record<string, string> | undefined;
    expect(deps).toBeDefined();
    expect(deps!['@crosstown/sdk']).toBeDefined();
  });

  it('should have @crosstown/relay as a runtime dependency', () => {
    const pkg = readJson(townPackagePath());
    const deps = pkg['dependencies'] as Record<string, string> | undefined;
    expect(deps!['@crosstown/relay']).toBeDefined();
  });

  it('should have @crosstown/core as a runtime dependency', () => {
    const pkg = readJson(townPackagePath());
    const deps = pkg['dependencies'] as Record<string, string> | undefined;
    expect(deps!['@crosstown/core']).toBeDefined();
  });

  it('should have hono as a runtime dependency (BLS HTTP server)', () => {
    const pkg = readJson(townPackagePath());
    const deps = pkg['dependencies'] as Record<string, string> | undefined;
    expect(deps!['hono']).toBeDefined();
  });

  it('should have @hono/node-server as a runtime dependency', () => {
    const pkg = readJson(townPackagePath());
    const deps = pkg['dependencies'] as Record<string, string> | undefined;
    expect(deps!['@hono/node-server']).toBeDefined();
  });

  it('should have better-sqlite3 as a runtime dependency (not devDep)', () => {
    // Story 2.5 Task 4: better-sqlite3 must be a runtime dependency because
    // startTown() creates SqliteEventStore at runtime.
    const pkg = readJson(townPackagePath());
    const deps = pkg['dependencies'] as Record<string, string> | undefined;
    const devDeps = pkg['devDependencies'] as
      | Record<string, string>
      | undefined;

    expect(
      deps!['better-sqlite3'],
      'better-sqlite3 must be in dependencies (not devDependencies)'
    ).toBeDefined();

    // Verify it's NOT only in devDependencies
    if (devDeps && devDeps['better-sqlite3']) {
      // It's OK to be in both, but it MUST be in dependencies
      expect(deps!['better-sqlite3']).toBeDefined();
    }
  });

  it('should have nostr-tools as a runtime dependency', () => {
    // Story 2.5 Task 4: nostr-tools needed by handler implementations at runtime.
    const pkg = readJson(townPackagePath());
    const deps = pkg['dependencies'] as Record<string, string> | undefined;
    expect(deps!['nostr-tools']).toBeDefined();
  });

  it('should have "type": "module" for ESM', () => {
    const pkg = readJson(townPackagePath());
    expect(pkg['type']).toBe('module');
  });

  it('should have a bin entry for CLI usage', () => {
    const pkg = readJson(townPackagePath());
    const bin = pkg['bin'] as Record<string, string> | string | undefined;
    expect(bin).toBeDefined();

    if (typeof bin === 'object') {
      const values = Object.values(bin);
      expect(values.length).toBeGreaterThan(0);
      // At least one bin entry should reference cli.js
      expect(values.some((v) => v.includes('cli.js'))).toBe(true);
    }
  });
});

// ============================================================================
// AC #1: TypeScript strict mode
// ============================================================================

describe('AC #1: TypeScript strict mode', () => {
  it('tsconfig.json should exist', () => {
    expect(existsSync(townTsconfigPath())).toBe(true);
  });

  it('tsconfig.json should extend root config or enable strict mode', () => {
    const tsconfig = readJson(townTsconfigPath());

    // Either extends the root tsconfig (which has strict: true) or has
    // strict: true directly
    const extendsRoot = typeof tsconfig['extends'] === 'string';
    const compilerOptions = tsconfig['compilerOptions'] as
      | Record<string, unknown>
      | undefined;
    const hasStrict = compilerOptions?.['strict'] === true;

    expect(
      extendsRoot || hasStrict,
      'tsconfig.json must extend root tsconfig (with strict:true) or set strict:true directly'
    ).toBe(true);
  });
});

// ============================================================================
// AC #6: npm publishability
// ============================================================================

describe('AC #6: npm publishability', () => {
  it('should have publishConfig.access set to "public"', () => {
    const pkg = readJson(townPackagePath());
    const publishConfig = pkg['publishConfig'] as
      | Record<string, string>
      | undefined;
    expect(publishConfig).toBeDefined();
    expect(publishConfig!['access']).toBe('public');
  });

  it('should have files field including dist/', () => {
    const pkg = readJson(townPackagePath());
    const files = pkg['files'] as string[] | undefined;
    expect(files).toBeDefined();
    expect(
      files!.some((f) => f === 'dist' || f === 'dist/' || f === 'dist/**'),
      'files field must include dist directory'
    ).toBe(true);
  });

  it('should have exports field with ESM entry', () => {
    const pkg = readJson(townPackagePath());
    const exports = pkg['exports'] as Record<string, unknown> | undefined;
    expect(exports).toBeDefined();

    // Should have a "." entry for the main module
    const mainExport = exports!['.'] as Record<string, string> | undefined;
    expect(mainExport).toBeDefined();

    // Should have an "import" field for ESM
    expect(mainExport!['import']).toBeDefined();
    expect(mainExport!['import']).toContain('index.js');

    // Should have a "types" field for TypeScript
    expect(mainExport!['types']).toBeDefined();
    expect(mainExport!['types']).toContain('index.d.ts');
  });

  it('should have main field pointing to dist/index.js', () => {
    const pkg = readJson(townPackagePath());
    expect(pkg['main']).toBe('./dist/index.js');
  });

  it('should have types field pointing to dist/index.d.ts', () => {
    const pkg = readJson(townPackagePath());
    expect(pkg['types']).toBe('./dist/index.d.ts');
  });

  it('should have correct package name', () => {
    const pkg = readJson(townPackagePath());
    expect(pkg['name']).toBe('@crosstown/town');
  });

  it('should have build script using tsup', () => {
    const pkg = readJson(townPackagePath());
    const scripts = pkg['scripts'] as Record<string, string> | undefined;
    expect(scripts).toBeDefined();
    expect(scripts!['build']).toContain('tsup');
  });
});

// ============================================================================
// tsup config: entry points
// ============================================================================

describe('tsup config: entry points', () => {
  it('tsup.config.ts should include both index.ts and cli.ts as entries', () => {
    const tsupConfigPath = resolve(
      repoRoot(),
      'packages',
      'town',
      'tsup.config.ts'
    );
    expect(existsSync(tsupConfigPath)).toBe(true);

    const source = readFileSync(tsupConfigPath, 'utf-8');

    // Should include index.ts entry
    expect(source).toContain('index.ts');

    // Should include cli.ts entry
    expect(source).toContain('cli.ts');
  });
});
