/**
 * `GET /health` reports VERSION, so VERSION must be the version that shipped.
 *
 * This guards the injection wiring rather than a hand-copied constant: if the
 * `define` is dropped from tsup.config.ts or vitest.config.ts, or the
 * placeholder is renamed on one side only, `VERSION` stops being a version
 * string and this fails.
 */

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { VERSION } from './version.js';

describe('VERSION', () => {
  it('is the version package.json declares', () => {
    const pkg = JSON.parse(
      readFileSync(resolve(import.meta.dirname, '../package.json'), 'utf8')
    ) as { version: string };

    expect(VERSION).toBe(pkg.version);
  });

  it('is a resolved semver string, not an unsubstituted placeholder', () => {
    expect(VERSION).toMatch(/^\d+\.\d+\.\d+/);
  });
});
