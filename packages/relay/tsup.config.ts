import { defineConfig } from 'tsup';
import { relayVersionDefine } from './version-define';

export default defineConfig({
  // Substitutes src/version.ts's placeholder from package.json.
  define: relayVersionDefine,
  // Named entries so the launcher CLI lands at dist/cli.js (not
  // dist/launcher/cli.js) for the `relay` bin and the "./cli" export.
  entry: {
    index: 'src/index.ts',
    cli: 'src/launcher/cli.ts',
    // Worker-thread verify entry (relay#85): worker_threads needs a real
    // compiled file on disk; the pool resolves dist/verify-worker.js.
    'verify-worker': 'src/crypto/verify-worker.ts',
  },
  format: ['esm'],
  dts: true,
  sourcemap: true,
  clean: true,
  outDir: 'dist',
  // cli.ts begins with `#!/usr/bin/env node`; tsup preserves the shebang and
  // marks dist/cli.js executable for the `relay` bin.
});
