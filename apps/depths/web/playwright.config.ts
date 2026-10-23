import { defineConfig, devices } from '@playwright/test'

import { autoWorkers } from '../../../ui/tests/support/workers.js'

/**
 * One project per user story (tasks.md T043), plus `shell` for the sign-in gate and transport.
 * Every story runs against `tests/support/fake-backend.ts`; nothing here talks to a real server.
 *
 * Run one story: `pnpm --filter @companion/depths-web e2e --project us1-stacks`.
 */
const STORIES = ['shell', 'us1-stacks', 'us2-lifecycle', 'us3-edit', 'us4-tabs', 'us5-access'] as const

/** `DEPTHS_E2E_PORT` moves the preview, so several worktrees can run a story at once. */
const PORT = Number(process.env.DEPTHS_E2E_PORT ?? 4183)
const ORIGIN = `http://localhost:${String(PORT)}`

export default defineConfig({
  fullyParallel: true,
  // Sized from the machine's current load and free memory (CLAUDE.md: one shared box);
  // `PLAYWRIGHT_WORKERS=<n>` overrides.
  workers: autoWorkers(),
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
    // The production bundle, as the server ships it: the fake backend intercepts its requests, so
    // the app has no fake mode. Built rather than served by the dev server, as in ui/.
    command:
      'pnpm exec vite build --outDir dist-e2e && ' +
      `pnpm exec vite preview --outDir dist-e2e --port ${String(PORT)} --strictPort`,
    url: ORIGIN,
    reuseExistingServer: false,
    timeout: 60_000,
  },
})
