import react from '@vitejs/plugin-react'
import { defineConfig } from 'vitest/config'

// Same pool rule as ui/vitest.config.ts: `scripts/ci.sh` nices every step and sets VITEST_WORKERS.
const WORKERS: number | undefined = process.env.VITEST_WORKERS
  ? Math.max(1, Number(process.env.VITEST_WORKERS))
  : undefined

export default defineConfig({
  plugins: [react()],
  test: {
    environment: 'jsdom',
    pool: 'forks',
    ...(WORKERS === undefined ? {} : { maxWorkers: WORKERS }),
    setupFiles: ['./tests/setup.ts'],
    include: ['tests/**/*.test.{ts,tsx}'],
    css: false,
    restoreMocks: true,
  },
})
