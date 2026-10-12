import { defineConfig } from 'vite';
import { svelte } from '@sveltejs/vite-plugin-svelte';

// Im Entwicklungsmodus leitet Vite WebSocket und API an das Rust-Backend weiter.
export default defineConfig({
  plugins: [svelte()],
  server: {
    proxy: {
      '/ws': { target: 'ws://localhost:8073', ws: true },
      '/api': 'http://localhost:8073',
      '/health': 'http://localhost:8073',
    },
  },
});
