/**
 * Unit Tests: NOSTR_SECRET_KEY identity alias support (issue #26)
 *
 * The published oblivious-mode relay image honors the connector's documented
 * env contract, which names the identity key `NOSTR_SECRET_KEY` (the same var
 * the BLS entrypoint uses). The relay CLI must accept it as an alias for
 * TOON_SECRET_KEY so the image is a drop-in for connector #221's compose.
 *
 * These are source-based assertions, mirroring fee-per-event-env.test.ts.
 */

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';

const CLI_SOURCE_PATH = resolve(import.meta.dirname, 'cli.ts');

describe('NOSTR_SECRET_KEY identity alias (issue #26)', () => {
  it('[P0] cli.ts should read NOSTR_SECRET_KEY from environment', () => {
    const source = readFileSync(CLI_SOURCE_PATH, 'utf-8');
    expect(source).toContain('NOSTR_SECRET_KEY');
  });

  it('[P0] cli.ts should fall back to NOSTR_SECRET_KEY after TOON_SECRET_KEY', () => {
    const source = readFileSync(CLI_SOURCE_PATH, 'utf-8');
    // TOON_SECRET_KEY is resolved before NOSTR_SECRET_KEY so it wins when both
    // are set (?? chain ordering).
    expect(source).toMatch(
      /TOON_SECRET_KEY'\][\s\S]*?\?\?[\s\S]*?NOSTR_SECRET_KEY/
    );
  });
});
