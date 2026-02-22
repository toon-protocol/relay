import { defineConfig } from 'tsup';

export default defineConfig({
  entry: ['src/index.ts', 'src/entrypoint.ts', 'src/entrypoint-bls-only.ts'],
  format: ['esm'],
  dts: false,  // Temporarily disabled due to type resolution issues
  sourcemap: true,
  clean: true,
  outDir: 'dist',
});
