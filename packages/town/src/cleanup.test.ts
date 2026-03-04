/**
 * Tests for Story 2.4: Remove packages/git-proxy & Document Reference
 *
 * NOT SKIPPED: These tests verify static conditions in the repository
 * (file existence, dependency references) and do not depend on @crosstown/sdk.
 * They can run immediately to guard against regressions.
 */

import { describe, it, expect } from 'vitest';
import { existsSync, readFileSync, readdirSync } from 'fs';
import { join, resolve } from 'path';

// ============================================================================
// Helpers
// ============================================================================

/**
 * Resolve a path relative to the repository root.
 * The test file lives at packages/town/src/cleanup.test.ts, so the repo
 * root is three directories up.
 */
function repoRoot(): string {
  return resolve(__dirname, '..', '..', '..');
}

/**
 * Read and parse a JSON file, returning undefined if it does not exist.
 */
function readJsonFile(filePath: string): Record<string, unknown> | undefined {
  try {
    const content = readFileSync(filePath, 'utf-8');
    return JSON.parse(content) as Record<string, unknown>;
  } catch {
    return undefined;
  }
}

/**
 * Collect all package.json files in the packages/ directory.
 */
function getAllPackageJsonPaths(): string[] {
  const packagesDir = join(repoRoot(), 'packages');
  const paths: string[] = [];

  try {
    const entries = readdirSync(packagesDir, { withFileTypes: true });
    for (const entry of entries) {
      if (entry.isDirectory()) {
        const pkgPath = join(packagesDir, entry.name, 'package.json');
        if (existsSync(pkgPath)) {
          paths.push(pkgPath);
        }
      }
    }
  } catch {
    // If packages/ directory doesn't exist, return empty
  }

  // Also check docker/package.json since it's in the workspace
  const dockerPkg = join(repoRoot(), 'docker', 'package.json');
  if (existsSync(dockerPkg)) {
    paths.push(dockerPkg);
  }

  return paths;
}

// ============================================================================
// Tests
// ============================================================================

describe('Story 2.4: packages/git-proxy removal', () => {
  it('should not have packages/git-proxy directory', () => {
    // P2: The packages/git-proxy directory must be removed as part of
    // the SDK consolidation. All git proxy functionality is superseded
    // by the NIP-34 handler in @crosstown/core.
    const gitProxyDir = join(repoRoot(), 'packages', 'git-proxy');
    expect(
      existsSync(gitProxyDir),
      `Expected packages/git-proxy to be removed, but it still exists at: ${gitProxyDir}`
    ).toBe(false);
  });

  it('should not have any package depending on @crosstown/git-proxy', () => {
    // P2: No workspace package should reference @crosstown/git-proxy in
    // dependencies, devDependencies, or peerDependencies.
    const packageJsonPaths = getAllPackageJsonPaths();
    expect(packageJsonPaths.length).toBeGreaterThan(0);

    const dependingPackages: string[] = [];

    for (const pkgPath of packageJsonPaths) {
      const pkg = readJsonFile(pkgPath);
      if (!pkg) continue;

      const depSections = [
        'dependencies',
        'devDependencies',
        'peerDependencies',
      ] as const;
      for (const section of depSections) {
        const deps = pkg[section] as Record<string, string> | undefined;
        if (deps && '@crosstown/git-proxy' in deps) {
          dependingPackages.push(`${pkgPath} (${section})`);
        }
      }
    }

    expect(
      dependingPackages,
      `Found packages depending on @crosstown/git-proxy:\n${dependingPackages.join('\n')}`
    ).toHaveLength(0);
  });

  it('should not reference @crosstown/git-proxy in pnpm-workspace.yaml', () => {
    // P2: The pnpm workspace config should not include git-proxy as
    // a workspace member after removal.
    const workspacePath = join(repoRoot(), 'pnpm-workspace.yaml');
    if (!existsSync(workspacePath)) {
      // If no workspace file, this condition is trivially satisfied
      return;
    }

    const content = readFileSync(workspacePath, 'utf-8');
    expect(
      content,
      'pnpm-workspace.yaml should not reference git-proxy'
    ).not.toMatch(/git-proxy/);
  });
});

describe('Story 2.4: SDK relay entrypoint', () => {
  it('SDK relay entrypoint should import from @crosstown/sdk (not manual wiring)', () => {
    // P2: The new SDK-based entrypoint (packages/town/src/index.ts or similar)
    // should import handler creation functions from @crosstown/sdk rather than
    // manually wiring BLS, SPSP server, and bootstrap logic.
    //
    // This test verifies that the SDK package exists and exports the expected
    // handler factories. It will FAIL until @crosstown/sdk is created.
    const sdkDir = join(repoRoot(), 'packages', 'sdk');
    const sdkPackageJson = join(sdkDir, 'package.json');

    expect(
      existsSync(sdkPackageJson),
      `Expected @crosstown/sdk package to exist at: ${sdkDir}`
    ).toBe(true);

    // Verify the package has the expected name
    const pkg = readJsonFile(sdkPackageJson);
    expect(pkg).toBeDefined();
    expect(pkg!['name']).toBe('@crosstown/sdk');
  });
});
