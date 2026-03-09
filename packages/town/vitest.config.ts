import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    environment: 'node',
    include: ['src/**/*.test.ts'],
    exclude: [
      '**/node_modules/**',
      '**/dist/**',
      'tests/e2e/**',
      // ATDD story tracker: exclude test files whose source modules do not yet exist.
      // Re-include each file as its corresponding story is implemented:
      //   event-storage-handler.test.ts    -> Story 2.1 (done)
      //   (spsp-handshake-handler removed in Story 2.7)
      //   subscribe.test.ts               -> Story 2.8 (RED phase -- all tests skipped)
    ],
  },
});
