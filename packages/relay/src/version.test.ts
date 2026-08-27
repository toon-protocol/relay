/**
 * `GET /health` reports VERSION, so VERSION must be the version that shipped.
 * A hand-maintained constant drifts silently; this is what stops it.
 */

import { describe, it, expect } from 'vitest';
import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { VERSION } from './version.js';

describe('VERSION', () => {
  it('matches the published package version', () => {
    const pkg = JSON.parse(
      readFileSync(resolve(import.meta.dirname, '../package.json'), 'utf8')
    ) as { version: string };

    expect(VERSION).toBe(pkg.version);
  });
});
