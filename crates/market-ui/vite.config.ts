import aurelia from '@aurelia/vite-plugin';
import { defineConfig } from 'vite';

export default defineConfig({
  plugins: [aurelia()],
  clearScreen: false,
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
    proxy: {
      '/api': 'http://127.0.0.1:8787',
      '/health': 'http://127.0.0.1:8787',
    },
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: { target: ['es2022', 'safari16'] },
});
