import { defineConfig } from 'tsup';

export default defineConfig({
  // Named entries so the launcher CLI lands at dist/cli.js (not
  // dist/launcher/cli.js) for the `relay` bin and the "./cli" export.
  entry: {
    index: 'src/index.ts',
    cli: 'src/launcher/cli.ts',
  },
  format: ['esm'],
  dts: true,
  sourcemap: true,
  clean: true,
  outDir: 'dist',
  // cli.ts begins with `#!/usr/bin/env node`; tsup preserves the shebang and
  // marks dist/cli.js executable for the `relay` bin.
});
