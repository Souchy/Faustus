import aurelia from '@aurelia/vite-plugin';
import { defineConfig } from 'vite';

const devApiUrl = process.env.FAUST_DEV_API_URL ?? 'http://127.0.0.1:8787';

export default defineConfig({
  plugins: [aurelia()],
  clearScreen: false,
  server: {
    host: '127.0.0.1',
    port: 5173,
    strictPort: true,
    proxy: {
      '/api': devApiUrl,
      '/health': devApiUrl,
    },
    watch: { ignored: ['**/src-tauri/**'] },
  },
  build: { target: ['es2022', 'safari16'] },
});
