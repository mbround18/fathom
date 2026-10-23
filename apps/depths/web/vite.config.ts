import { fileURLToPath, URL } from 'node:url'

import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  server: {
    port: 5174,
    strictPort: true,
    // `make depths-dev` runs the server on 8080; the browser talks to it through Vite.
    proxy: {
      '/api': { target: 'http://127.0.0.1:8080', ws: true },
      '/auth': 'http://127.0.0.1:8080',
    },
  },
})
