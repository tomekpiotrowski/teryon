import { defineConfig } from 'vitest/config';
import { fileURLToPath } from 'node:url';

export default defineConfig({
  resolve: {
    alias: {
      'erno-angular': fileURLToPath(new URL('./src/testing/erno-angular.mock.ts', import.meta.url)),
    },
  },
});
