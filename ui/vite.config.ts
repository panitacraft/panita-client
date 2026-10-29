import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import desktopConfig from '../desktop/tauri.conf.json' with { type: 'json' };

export default defineConfig({
  define: { __APP_VERSION__: JSON.stringify(desktopConfig.version) },
  resolve: { alias: { '@': decodeURIComponent(new URL('./src', import.meta.url).pathname).replace(/^\/(\w:)/, '$1') } },
  plugins: [react(), tailwindcss()],
  clearScreen: false,
  server: { host: '127.0.0.1', port: 1420, strictPort: true },
});
