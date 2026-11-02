import tailwindcss from '@tailwindcss/vite'
import react from '@vitejs/plugin-react'
import { defineConfig } from 'vite'

/**
 * The kit's isolated test harness (spec 065, contracts/harness.md): one page that renders one
 * piece in one state, with Keel's CSS, for the per-piece Playwright specs and for looking at a
 * piece by hand (`pnpm --filter @companion/ui harness`).
 */
export default defineConfig({
  root: 'harness',
  plugins: [react(), tailwindcss()],
  server: { port: Number(process.env.KIT_E2E_PORT ?? 4190), strictPort: true },
})
