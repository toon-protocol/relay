import { defineConfig } from 'vitest/config';

// Deliberately a separate config, and a script that is NOT called `test`:
// this suite needs a container runtime and a built relay image, so it must
// not run in `pnpm -r test`. CI runs it in its own job (`conformance`).
export default defineConfig({
  test: {
    globals: true,
    environment: 'node',
    include: ['suite/**/*.test.ts'],
    // Booting an image can include a cold start, and most cases boot their
    // own relay inside the test (up to 60s for /health, then 30s for the
    // document), so a test must outlast the harness's own deadlines.
    testTimeout: 120_000,
    hookTimeout: 120_000,
  },
});
