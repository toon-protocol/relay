import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    globals: true,
    environment: 'node',
    testTimeout: 120_000,
    pool: 'forks',
    poolOptions: {
      forks: { minForks: 1, maxForks: 4 },
    },
    // The guards that read the real deploy artifacts, the workflows and the
    // Rust workspace's files; they live next to the files they guard.
    include: ['deploy/*.test.ts'],
    exclude: ['**/node_modules/**', '**/dist/**'],
  },
});
