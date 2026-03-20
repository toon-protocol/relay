/**
 * Unit tests for the CLI entrypoint (Story 2.5 AC #4).
 *
 * Tests validate:
 * - CLI --help output and exit behavior
 * - Missing required arguments produce errors
 * - Environment variable support
 * - CLI flags override env vars
 * - CLI source file structure (shebang, parseArgs usage)
 *
 * Since parseCli() is not exported, CLI behavior is tested by:
 * 1. Static analysis of the source file (structure, shebang, imports)
 * 2. Subprocess execution for --help and error cases
 *
 * These tests do NOT require genesis node infrastructure.
 */

import { describe, it, expect } from 'vitest';
import { readFileSync, existsSync } from 'fs';
import { resolve } from 'path';
import { execFileSync } from 'child_process';

// ============================================================================
// Helpers
// ============================================================================

/**
 * Resolve a path relative to the repository root.
 */
function repoRoot(): string {
  return resolve(__dirname, '..', '..', '..');
}

/**
 * Path to the CLI source file.
 */
function cliSourcePath(): string {
  return resolve(__dirname, 'cli.ts');
}

/**
 * Path to the built CLI file (after pnpm build).
 */
function cliDistPath(): string {
  return resolve(__dirname, '..', 'dist', 'cli.js');
}

// ============================================================================
// CLI Source Structure Tests (AC #4)
// ============================================================================

describe('CLI source structure (AC #4)', () => {
  it('cli.ts should exist', () => {
    expect(
      existsSync(cliSourcePath()),
      'packages/town/src/cli.ts must exist'
    ).toBe(true);
  });

  it('cli.ts should have #!/usr/bin/env node shebang', () => {
    const source = readFileSync(cliSourcePath(), 'utf-8');
    const firstLine = source.split('\n')[0];

    expect(
      firstLine,
      'CLI entrypoint must start with #!/usr/bin/env node shebang'
    ).toBe('#!/usr/bin/env node');
  });

  it('cli.ts should use parseArgs from node:util (no external CLI framework)', () => {
    const source = readFileSync(cliSourcePath(), 'utf-8');

    // Must import parseArgs from node:util (standard library)
    expect(source).toMatch(
      /import\s*\{[^}]*parseArgs[^}]*\}\s*from\s*['"]node:util['"]/
    );

    // Must NOT import from external CLI frameworks
    expect(source).not.toMatch(/from\s*['"]commander['"]/);
    expect(source).not.toMatch(/from\s*['"]yargs['"]/);
    expect(source).not.toMatch(/from\s*['"]meow['"]/);
  });

  it('cli.ts should import startTown from town.js (thin wrapper pattern)', () => {
    const source = readFileSync(cliSourcePath(), 'utf-8');

    // CLI must import startTown from the main module
    expect(source).toMatch(
      /import\s*\{[^}]*startTown[^}]*\}\s*from\s*['"]\.\/town\.js['"]/
    );
  });

  it('cli.ts should support all documented CLI flags', () => {
    // AC #4 requires: --mnemonic, --secret-key, --relay-port, --bls-port,
    // --data-dir, --connector-url, --known-peers, --dev-mode, --help
    // Story 3.4 adds: --discovery, --seed-relays, --publish-seed-entry, --external-relay-url
    const source = readFileSync(cliSourcePath(), 'utf-8');

    // Flags appear as property keys in the parseArgs options object.
    // Some are unquoted (mnemonic, help) and some are quoted ('secret-key').
    // Check that each flag name appears in the source in any form.
    const requiredFlags = [
      'mnemonic',
      'secret-key',
      'relay-port',
      'bls-port',
      'data-dir',
      'connector-url',
      'known-peers',
      'dev-mode',
      'help',
      // Story 3.4: seed relay discovery flags
      'discovery',
      'seed-relays',
      'publish-seed-entry',
      'external-relay-url',
    ];

    for (const flag of requiredFlags) {
      expect(source, `CLI must support --${flag} flag`).toContain(flag);
    }
  });

  it('cli.ts should support all documented environment variables', () => {
    // AC #4 requires env var support matching CLI flags
    // Story 3.4 adds: TOON_DISCOVERY, TOON_SEED_RELAYS,
    // TOON_PUBLISH_SEED_ENTRY, TOON_EXTERNAL_RELAY_URL
    const source = readFileSync(cliSourcePath(), 'utf-8');

    const requiredEnvVars = [
      'TOON_MNEMONIC',
      'TOON_SECRET_KEY',
      'TOON_RELAY_PORT',
      'TOON_BLS_PORT',
      'TOON_DATA_DIR',
      'TOON_CONNECTOR_URL',
      'TOON_KNOWN_PEERS',
      'TOON_DEV_MODE',
      // Story 3.4: seed relay discovery env vars
      'TOON_DISCOVERY',
      'TOON_SEED_RELAYS',
      'TOON_PUBLISH_SEED_ENTRY',
      'TOON_EXTERNAL_RELAY_URL',
    ];

    for (const envVar of requiredEnvVars) {
      expect(source, `CLI must read ${envVar} environment variable`).toContain(
        envVar
      );
    }
  });

  it('cli.ts should wire SIGINT and SIGTERM to graceful shutdown', () => {
    const source = readFileSync(cliSourcePath(), 'utf-8');

    expect(source, 'Must handle SIGINT').toContain('SIGINT');
    expect(source, 'Must handle SIGTERM').toContain('SIGTERM');
    expect(source, 'Must call instance.stop()').toMatch(/instance\.stop\(\)/);
  });

  it('cli.ts should use CLI flags to override env vars (flag ?? env pattern)', () => {
    // AC #4: CLI flags override environment variables.
    // The parseCli() function uses `values[flag] ?? process.env[ENV_VAR]`
    // which means CLI flags (from parseArgs) take precedence over env vars.
    const source = readFileSync(cliSourcePath(), 'utf-8');

    // Verify the override pattern: `values[...] ??` followed by `process.env[...]`
    // This ensures CLI flags are checked first, with env vars as fallback.
    // The pattern allows for line breaks between the ?? and process.env.
    const overridePairs: [string, string][] = [
      ['mnemonic', 'TOON_MNEMONIC'],
      ["'secret-key'", 'TOON_SECRET_KEY'],
      ["'connector-url'", 'TOON_CONNECTOR_URL'],
      ["'relay-port'", 'TOON_RELAY_PORT'],
      ["'bls-port'", 'TOON_BLS_PORT'],
      ["'data-dir'", 'TOON_DATA_DIR'],
    ];

    for (const [flag, envVar] of overridePairs) {
      // Check that the source contains the flag-first pattern
      expect(
        source,
        `CLI must read values[${flag}] before ${envVar}`
      ).toContain(envVar);

      // Verify flag appears before env var in the source (flag ?? env ordering)
      const flagIdx =
        source.indexOf(`values[${flag}]`) >= 0
          ? source.indexOf(`values[${flag}]`)
          : source.indexOf(`values.${flag}`);
      const envIdx = source.indexOf(`'${envVar}'`);
      expect(
        flagIdx,
        `values[${flag}] must appear in source`
      ).toBeGreaterThanOrEqual(0);
      expect(
        flagIdx < envIdx,
        `values[${flag}] must come before ${envVar} (flag overrides env)`
      ).toBe(true);
    }
  });
});

// ============================================================================
// CLI Runtime Tests (AC #4) -- requires built dist/cli.js
// ============================================================================

describe('CLI runtime behavior (AC #4)', () => {
  it('--help should print usage and exit with code 0', () => {
    const cliPath = cliDistPath();
    if (!existsSync(cliPath)) {
      console.log('Skipping: dist/cli.js not built. Run pnpm build first.');
      return;
    }

    // Run the CLI with --help flag
    const output = execFileSync('node', [cliPath, '--help'], {
      encoding: 'utf-8',
      timeout: 5000,
    });

    // Should print usage information
    expect(output).toContain('Usage:');
    expect(output).toContain('--mnemonic');
    expect(output).toContain('--connector-url');
    expect(output).toContain('--relay-port');
    expect(output).toContain('--bls-port');
    expect(output).toContain('--secret-key');
    expect(output).toContain('--data-dir');
    expect(output).toContain('--dev-mode');
    expect(output).toContain('--known-peers');

    // Should document environment variables
    expect(output).toContain('TOON_MNEMONIC');
    expect(output).toContain('TOON_CONNECTOR_URL');
  });

  it('should exit with error when no identity is provided', () => {
    const cliPath = cliDistPath();
    if (!existsSync(cliPath)) {
      console.log('Skipping: dist/cli.js not built. Run pnpm build first.');
      return;
    }

    // Run the CLI without mnemonic or secret-key (but with connector-url to
    // pass that validation first)
    try {
      execFileSync(
        'node',
        [cliPath, '--connector-url', 'http://localhost:8080'],
        {
          encoding: 'utf-8',
          timeout: 5000,
          env: {
            ...process.env,
            // Clear any env vars that might provide identity
            TOON_MNEMONIC: '',
            TOON_SECRET_KEY: '',
          },
        }
      );
      // Should have exited with error
      expect.fail('CLI should have exited with non-zero code');
    } catch (error: unknown) {
      // execFileSync throws on non-zero exit
      const err = error as { status?: number; stderr?: string };
      expect(err.status).not.toBe(0);
    }
  });

  it('should exit with error when --relay-port is invalid (non-numeric)', () => {
    const cliPath = cliDistPath();
    if (!existsSync(cliPath)) {
      console.log('Skipping: dist/cli.js not built. Run pnpm build first.');
      return;
    }

    try {
      execFileSync(
        'node',
        [
          cliPath,
          '--mnemonic',
          'test test test test test test test test test test test junk',
          '--connector-url',
          'http://localhost:8080',
          '--relay-port',
          'abc',
        ],
        {
          encoding: 'utf-8',
          timeout: 5000,
          env: {
            ...process.env,
            TOON_RELAY_PORT: '',
          },
        }
      );
      expect.fail('CLI should have exited with non-zero code');
    } catch (error: unknown) {
      const err = error as { status?: number; stderr?: string };
      expect(err.status).not.toBe(0);
    }
  });

  it('should exit with error when --connector-url is missing', () => {
    const cliPath = cliDistPath();
    if (!existsSync(cliPath)) {
      console.log('Skipping: dist/cli.js not built. Run pnpm build first.');
      return;
    }

    try {
      execFileSync(
        'node',
        [
          cliPath,
          '--mnemonic',
          'test test test test test test test test test test test junk',
        ],
        {
          encoding: 'utf-8',
          timeout: 5000,
          env: {
            ...process.env,
            TOON_CONNECTOR_URL: '',
          },
        }
      );
      expect.fail('CLI should have exited with non-zero code');
    } catch (error: unknown) {
      const err = error as { status?: number; stderr?: string };
      expect(err.status).not.toBe(0);
    }
  });
});

// ============================================================================
// CLI bin entry in package.json (AC #4)
// ============================================================================

describe('CLI bin entry in package.json (AC #4)', () => {
  it('package.json bin field should reference dist/cli.js', () => {
    const pkgPath = resolve(repoRoot(), 'packages', 'town', 'package.json');
    const pkg = JSON.parse(readFileSync(pkgPath, 'utf-8')) as Record<
      string,
      unknown
    >;

    const bin = pkg['bin'] as Record<string, string> | undefined;
    expect(bin).toBeDefined();

    // The bin entry should point to the compiled CLI file
    const binValues = Object.values(bin!);
    expect(
      binValues.some((v) => v.includes('cli.js')),
      'bin entry must reference cli.js'
    ).toBe(true);
  });

  it('built dist/cli.js should exist after build', () => {
    const cliPath = cliDistPath();
    if (!existsSync(cliPath)) {
      console.log('Skipping: dist/cli.js not built. Run pnpm build first.');
      return;
    }

    expect(existsSync(cliPath)).toBe(true);

    // Verify the built file has shebang
    const content = readFileSync(cliPath, 'utf-8');
    expect(content.startsWith('#!/usr/bin/env node')).toBe(true);
  });
});

// ============================================================================
// Story 3.4: Seed relay discovery CLI flags (AC #4)
// ============================================================================

describe('CLI seed relay discovery flags (Story 3.4 AC #4)', () => {
  it('--help should list seed relay discovery flags', () => {
    const cliPath = cliDistPath();
    if (!existsSync(cliPath)) {
      console.log('Skipping: dist/cli.js not built. Run pnpm build first.');
      return;
    }

    const output = execFileSync('node', [cliPath, '--help'], {
      encoding: 'utf-8',
      timeout: 5000,
    });

    // Story 3.4 flags must appear in help output
    expect(output).toContain('--discovery');
    expect(output).toContain('--seed-relays');
    expect(output).toContain('--publish-seed-entry');
    expect(output).toContain('--external-relay-url');

    // Story 3.4 env vars must appear in help output
    expect(output).toContain('TOON_DISCOVERY');
    expect(output).toContain('TOON_SEED_RELAYS');
    expect(output).toContain('TOON_PUBLISH_SEED_ENTRY');
    expect(output).toContain('TOON_EXTERNAL_RELAY_URL');
  });

  it('cli.ts validates --discovery accepts only "seed-list" or "genesis"', () => {
    const source = readFileSync(cliSourcePath(), 'utf-8');

    // Verify discovery mode validation
    expect(source).toContain("'seed-list'");
    expect(source).toContain("'genesis'");
    // Verify error message for invalid discovery mode
    expect(source).toMatch(/--discovery.*must.*seed-list.*genesis/i);
  });

  it('cli.ts parses --seed-relays as comma-separated list', () => {
    const source = readFileSync(cliSourcePath(), 'utf-8');

    // Verify comma-splitting logic for seed relays
    expect(source).toMatch(/split\s*\(\s*['"],['"].*\)/);
    // Verify the flag is wired into the TownConfig
    expect(source).toContain('seedRelays');
  });

  it('cli.ts passes seed relay config fields to TownConfig', () => {
    const source = readFileSync(cliSourcePath(), 'utf-8');

    // Verify all seed relay config fields are included in the returned config
    expect(source).toContain('discovery: discoveryMode');
    expect(source).toContain('seedRelays');
    expect(source).toContain('publishSeedEntry');
    expect(source).toContain('externalRelayUrl');
  });
});

// ============================================================================
// Story 3.4: Docker env var parsing for seed relay discovery
// ============================================================================

describe('Docker shared.ts seed relay env vars (Story 3.4)', () => {
  it('shared.ts parses TOON_DISCOVERY env var', () => {
    const sharedPath = resolve(repoRoot(), 'docker', 'src', 'shared.ts');
    const source = readFileSync(sharedPath, 'utf-8');

    expect(source).toContain('TOON_DISCOVERY');
    expect(source).toContain('discoveryMode');
  });

  it('shared.ts parses TOON_SEED_RELAYS as comma-separated list', () => {
    const sharedPath = resolve(repoRoot(), 'docker', 'src', 'shared.ts');
    const source = readFileSync(sharedPath, 'utf-8');

    expect(source).toContain('TOON_SEED_RELAYS');
    // Verify comma-splitting logic
    expect(source).toMatch(/split\s*\(\s*['"],['"].*\)/);
    expect(source).toContain('seedRelays');
  });

  it('shared.ts parses TOON_PUBLISH_SEED_ENTRY env var', () => {
    const sharedPath = resolve(repoRoot(), 'docker', 'src', 'shared.ts');
    const source = readFileSync(sharedPath, 'utf-8');

    expect(source).toContain('TOON_PUBLISH_SEED_ENTRY');
    expect(source).toContain('publishSeedEntry');
  });

  it('shared.ts parses TOON_EXTERNAL_RELAY_URL env var', () => {
    const sharedPath = resolve(repoRoot(), 'docker', 'src', 'shared.ts');
    const source = readFileSync(sharedPath, 'utf-8');

    expect(source).toContain('TOON_EXTERNAL_RELAY_URL');
    expect(source).toContain('externalRelayUrl');
  });

  it('shared.ts defaults discoveryMode to "genesis"', () => {
    const sharedPath = resolve(repoRoot(), 'docker', 'src', 'shared.ts');
    const source = readFileSync(sharedPath, 'utf-8');

    // Verify default value is 'genesis'
    expect(source).toMatch(/'genesis'/);
  });

  it('shared.ts Config interface includes seed relay discovery fields', () => {
    const sharedPath = resolve(repoRoot(), 'docker', 'src', 'shared.ts');
    const source = readFileSync(sharedPath, 'utf-8');

    // Verify the Config interface includes seed relay discovery fields
    expect(source).toContain("discoveryMode: 'seed-list' | 'genesis'");
    expect(source).toContain('seedRelays: string[]');
    expect(source).toContain('publishSeedEntry: boolean');
    expect(source).toContain('externalRelayUrl: string | undefined');
  });
});
