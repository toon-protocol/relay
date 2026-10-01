import { defineConfig } from 'vitest/config';

// Deliberately a separate config, and a script that is NOT called `test`:
// this suite needs a container runtime and a built relay image, so it must
// not run in `pnpm -r test`. CI runs it in its own job (`conformance`).
export default defineConfig({
  test: {
    globals: true,
    environment: 'node',
    include: ['suite/**/*.test.ts'],
    // Booting an image can include a cold start; tests themselves are quick.
    testTimeout: 30_000,
    hookTimeout: 120_000,
  },
});
