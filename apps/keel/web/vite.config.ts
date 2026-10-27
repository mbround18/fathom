import { fileURLToPath, URL } from 'node:url'

import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  server: {
    port: 5175,
    strictPort: true,
    // The Keel server listens on 7830 (contracts/config.md); the browser talks to it through Vite.
    proxy: {
      '/api': { target: 'http://127.0.0.1:7830', ws: true },
      '/auth': 'http://127.0.0.1:7830',
    },
  },
})
