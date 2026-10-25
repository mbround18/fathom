import { defineConfig } from 'vitest/config'

// Same pool rule as ui/vitest.config.ts: `scripts/ci.sh` nices every step and sets VITEST_WORKERS.
const WORKERS: number | undefined = process.env.VITEST_WORKERS
  ? Math.max(1, Number(process.env.VITEST_WORKERS))
  : undefined

export default defineConfig({
  test: {
    environment: 'node',
    pool: 'forks',
    ...(WORKERS === undefined ? {} : { maxWorkers: WORKERS }),
    include: ['tests/**/*.test.{ts,tsx}'],
    restoreMocks: true,
  },
})
