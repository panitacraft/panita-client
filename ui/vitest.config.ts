import { defineConfig } from 'vitest/config';
import react from '@vitejs/plugin-react';

export default defineConfig({
  plugins: [react()],
  resolve: { alias: { '@': decodeURIComponent(new URL('./src', import.meta.url).pathname).replace(/^\/(\w:)/, '$1') } },
  test: { environment: 'happy-dom', include: ['tests/**/*.test.tsx'], clearMocks: true, pool: 'threads', maxWorkers: 1 },
});
