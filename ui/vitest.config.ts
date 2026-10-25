import { fileURLToPath, URL } from 'node:url'

import react from '@vitejs/plugin-react'
import { defineConfig } from 'vitest/config'

/**
 * Fork-pool size. Unset means vitest's own default of one fork per core, which is right for an
 * interactive run; `scripts/ci.sh` sets it because it nices every step (see `maxWorkers` below).
 */
const WORKERS: number | undefined = process.env.VITEST_WORKERS
  ? Math.max(1, Number(process.env.VITEST_WORKERS))
  : undefined

export default defineConfig({
  plugins: [react()],
  define: { __APP_VERSION__: JSON.stringify('9.8.7-test') },
  resolve: {
    alias: { '@': fileURLToPath(new URL('./src', import.meta.url)) },
  },
  test: {
    environment: 'jsdom',
    // `scripts/ci.sh` runs every step at `nice -n 10` to keep the dev box usable, and sets
    // `VITEST_WORKERS`. The default pool is one fork per core, so 31 niced workers start at once
    // against an unniced dagger engine and a few of them miss tinypool's startup handshake: the
    // run fails with "Failed to start forks worker" while every test that did run passes, which
    // reads like flake and is not. A smaller pool is both politer than 31 processes and immune to
    // the race — measured, it also ran 3x faster.
    pool: 'forks',
    ...(WORKERS === undefined ? {} : { maxWorkers: WORKERS }),
    setupFiles: ['./tests/unit/setup.ts'],
    // The theme package's contrast suite guards Fathom's own class names too (it scans ui/src).
    include: ['tests/unit/**/*.test.{ts,tsx}', '../packages/theme/tests/**/*.test.{ts,tsx}'],
    css: false,
    restoreMocks: true,
  },
})
