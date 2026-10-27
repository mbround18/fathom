import { defineConfig, devices } from '@playwright/test'

import { autoWorkers } from '../../../ui/tests/support/workers.js'

/**
 * One project per user story (spec 055 tasks.md T010), plus `shell` for the sign-in gate and
 * transport. Every story runs against the fake backend; nothing here talks to a real server.
 *
 * Run one story: `pnpm --filter @companion/keel-web e2e --project us1`.
 */
const STORIES = ['shell', 'us1', 'us2', 'us3', 'us4', 'us5', 'us6', 'us7', 'us8', 'us9'] as const

/** `KEEL_E2E_PORT` moves the preview, so several worktrees can run a story at once. */
const PORT = Number(process.env.KEEL_E2E_PORT ?? 4184)
const ORIGIN = `http://localhost:${String(PORT)}`

const workers = autoWorkers()

export default defineConfig({
  fullyParallel: true,
  // Sized from the machine's current load and free memory (CLAUDE.md: one shared box);
  // `PLAYWRIGHT_WORKERS=<n>` overrides.
  ...(workers === undefined ? {} : { workers }),
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 2 : 0,
  reporter: [['list'], ...(process.env.CI ? [['html', { open: 'never' }] as const] : [])],
  use: {
    baseURL: ORIGIN,
    trace: 'retain-on-failure',
  },
  projects: STORIES.map((story) => ({
    name: story,
    testDir: `./tests/e2e/${story}`,
    use: { ...devices['Desktop Chrome'] },
  })),
  webServer: {
    // The production bundle, as the server ships it. Built rather than served by the dev server.
    command:
      'pnpm exec vite build --outDir dist-e2e && ' +
      `pnpm exec vite preview --outDir dist-e2e --port ${String(PORT)} --strictPort`,
    url: ORIGIN,
    reuseExistingServer: false,
    timeout: 60_000,
  },
})
