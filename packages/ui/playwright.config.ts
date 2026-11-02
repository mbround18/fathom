import { defineConfig, devices } from '@playwright/test'

import { autoWorkers } from '../../ui/tests/support/workers.js'

/**
 * The kit's isolated piece specs (spec 065, contracts/harness.md): each piece alone in the harness,
 * no app and no server. Run one piece: `pnpm --filter @companion/ui e2e tests/e2e/<id>.spec.ts`.
 */

/** `KIT_E2E_PORT` moves the harness, so several worktrees can run the specs at once. */
const PORT = Number(process.env.KIT_E2E_PORT ?? 4190)
const ORIGIN = `http://localhost:${String(PORT)}`

const workers = autoWorkers()

export default defineConfig({
  testDir: './tests/e2e',
  fullyParallel: true,
  // Sized from the machine's current load and free memory; `PLAYWRIGHT_WORKERS=<n>` overrides.
  ...(workers === undefined ? {} : { workers }),
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  reporter: [['list'], ...(process.env.CI ? [['html', { open: 'never' }] as const] : [])],
  use: { baseURL: ORIGIN, trace: 'retain-on-failure' },
  projects: [{ name: 'kit', use: { ...devices['Desktop Chrome'] } }],
  webServer: {
    command: `pnpm exec vite --config vite.harness.config.ts --port ${String(PORT)} --strictPort`,
    url: ORIGIN,
    reuseExistingServer: false,
    timeout: 60_000,
  },
})
