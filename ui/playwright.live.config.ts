import { defineConfig, devices } from '@playwright/test'

/**
 * Opt-in LIVE suite: the real UI (a `--mode live` bundle, ws transport) against the real Rust
 * backend through `tools/devbridge`, against a real cluster, confined to the `fathom-dev`
 * sandbox namespace. Run with:
 *
 *   FATHOM_LIVE_CONTEXT=<kube-context> pnpm -C ui e2e:live
 *
 * Kept apart from playwright.config.ts (testDir tests/e2e) so `pnpm e2e` never runs it. The
 * global setup refuses to start unless `FATHOM_LIVE_CONTEXT` is set and its `fathom-dev`
 * namespace carries `fathom.dev/sandbox=true`; it then starts the bridge, and the specs reach it
 * through the `app` fixture (tests/live/helpers.ts).
 */
const APP_ORIGIN = 'http://localhost:4174'

export default defineConfig({
  testDir: './tests/live',
  // The specs share one real namespace (and one backend): strictly one at a time.
  workers: 1,
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  timeout: 90_000,
  expect: { timeout: 15_000 },
  reporter: 'list',
  globalSetup: './tests/live/global-setup.ts',
  globalTeardown: './tests/live/global-teardown.ts',
  use: {
    baseURL: APP_ORIGIN,
    trace: 'retain-on-failure',
  },
  projects: [
    {
      name: 'chromium-phone',
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 390, height: 844 },
        deviceScaleFactor: 1,
        hasTouch: true,
      },
    },
    {
      name: 'chromium-wide',
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1920, height: 1080 },
        deviceScaleFactor: 1,
      },
    },
  ],
  webServer: {
    command: 'pnpm e2e:live:serve',
    url: APP_ORIGIN,
    reuseExistingServer: false,
    timeout: 120_000,
  },
})
