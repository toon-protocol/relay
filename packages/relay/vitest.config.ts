import { defineConfig } from 'vitest/config';
import { relayVersionDefine } from './version-define';

export default defineConfig({
  define: relayVersionDefine,
  test: {
    globals: true,
    environment: 'node',
    include: ['src/**/*.test.ts'],
    exclude: ['**/node_modules/**', '**/dist/**'],
  },
});
