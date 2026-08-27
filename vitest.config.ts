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
    // The relay package's own suites, plus deploy/bundle.test.ts — the guard
    // that reads the real deploy artifacts (it is not relay source, so it
    // lives next to the files it guards).
    include: ['packages/*/src/**/*.test.ts', 'deploy/*.test.ts'],
    exclude: ['**/node_modules/**', '**/dist/**'],
    coverage: {
      provider: 'v8',
      reporter: ['text', 'json', 'html'],
      include: ['packages/*/src/**/*.ts'],
      exclude: [
        '**/node_modules/**',
        '**/dist/**',
        '**/*.test.ts',
        '**/index.ts',
      ],
    },
  },
});
