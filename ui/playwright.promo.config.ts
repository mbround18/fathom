import { defineConfig, devices } from '@playwright/test'

import { APP_ORIGIN, LIVE_SERVE_COMMAND } from './tests/live/env.js'

/**
 * Promo capture: the real UI against the real backend (same devbridge and `--mode live` bundle as
 * the live suite), photographing the `fathom-dev` sandbox and composing the shots into a branded
 * PDF. READ-ONLY: every write command the page tries to send is dropped before it reaches the
 * bridge, and the run fails if one was attempted (tests/promo/helpers.ts).
 *
 *   FATHOM_LIVE_CONTEXT=<kube-context> pnpm -C ui promo
 *
 * Output: ui/promo-out/ (fathom-promo.pdf, brochure.html, shots/*.png, facts.json).
 */

export default defineConfig({
  testDir: './tests/promo',
  // Scenes run in order and the last one composes the PDF from the shots before it.
  workers: 1,
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  // A shaky cluster read retries the scene; the composer checks every shot exists.
  retries: 1,
  timeout: 120_000,
  expect: { timeout: 20_000 },
  reporter: 'list',
  globalSetup: './tests/live/global-setup.ts',
  globalTeardown: './tests/live/global-teardown.ts',
  use: {
    ...devices['Desktop Chrome'],
    baseURL: APP_ORIGIN,
    colorScheme: 'dark',
    reducedMotion: 'reduce',
    locale: 'en-US',
    timezoneId: 'UTC',
    trace: 'retain-on-failure',
  },
  projects: [{ name: 'promo' }],
  webServer: {
    command: LIVE_SERVE_COMMAND,
    url: APP_ORIGIN,
    reuseExistingServer: false,
    timeout: 120_000,
  },
})
