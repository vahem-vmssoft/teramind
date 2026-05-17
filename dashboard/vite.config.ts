import { defineConfig } from 'vite';
import react from '@vitejs/plugin-react';
import path from 'node:path';

export default defineConfig({
  test: {
    environment: 'node',
    include: ['tests/**/*.test.ts'],
  },
  plugins: [react()],
  base: '/dashboard/',
  server: {
    port: 5173,
    proxy: {
      '/admin':     { target: 'http://localhost:8443', changeOrigin: true },
      // WebSocket proxy for /admin/events
      '/admin/events': { target: 'ws://localhost:8443', ws: true, changeOrigin: true },
    },
  },
  resolve: { alias: { '@': path.resolve(__dirname, 'src') } },
  build: {
    target: 'es2022',
    sourcemap: false,
    rollupOptions: {
      output: {
        manualChunks: { recharts: ['recharts'] },
      },
    },
  },
});
