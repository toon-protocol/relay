import { defineConfig } from 'tsup';

export default defineConfig({
  entry: ['src/index.ts', 'src/entrypoint.ts'],
  format: ['esm'],
  dts: true,
  sourcemap: true,
  clean: true,
  outDir: 'dist',
});
