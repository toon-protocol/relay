// The relay's version, as a build-time substitution.
//
// `package.json` is the only place the version is written: `changeset version`
// bumps it and nothing else. Anything that compiles or runs src/version.ts
// substitutes `__RELAY_VERSION__` from here, so a released bundle reports the
// version it was cut from and a hand-copied constant can never drift out of
// step with it (`GET /health` once served a hardcoded 0.1.0 from a 2.0.2
// build).
//
// Three consumers, all importing this one file rather than re-reading
// package.json themselves: packages/relay/tsup.config.ts (the shipped bundle),
// packages/relay/vitest.config.ts and the root vitest.config.ts (both test
// runners). Plain ESM so every one of them can import it without a build step.

import { readFileSync } from 'node:fs';

const { version } = JSON.parse(
  readFileSync(new URL('./package.json', import.meta.url), 'utf8')
) as { version: string };

/** esbuild/vite `define` entry substituting src/version.ts's placeholder. */
export const relayVersionDefine: Record<string, string> = {
  __RELAY_VERSION__: JSON.stringify(version),
};
