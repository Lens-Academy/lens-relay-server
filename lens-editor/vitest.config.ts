import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';

export default defineConfig({
  plugins: [react(), tailwindcss()],
  test: {
    environment: 'happy-dom',
    globals: true,
    include: ['src/**/*.test.{ts,tsx}', 'server/**/*.test.ts'],
    setupFiles: ['src/test/setup.ts'],
    environmentMatchGlobs: [
      ['server/**/*.test.ts', 'node'],
    ],
    coverage: {
      provider: 'v8',
      include: ['src/lib/**', 'src/components/**/extensions/**', 'src/hooks/**'],
      exclude: ['**/*.test.ts', 'src/test/**'],
    },
  },
});
