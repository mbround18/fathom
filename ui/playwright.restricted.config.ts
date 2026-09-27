import { defineConfig, devices } from '@playwright/test'

import { APP_ORIGIN, LIVE_SERVE_COMMAND, restrictedKubeconfig } from './tests/live/env.js'

/**
 * Opt-in RESTRICTED-USER walk: the live UI against the real backend as a namespace-scoped
 * identity (k8s/samples/restricted-user.yaml: edit in fathom-dev + demo, view in default,
 * nothing cluster-scoped). Every nav item must work, or show a clean "Not permitted" state.
 *
 *   FATHOM_RESTRICTED_KUBECONFIG=<kubeconfig> FATHOM_LIVE_CONTEXT=fathom-restricted \
 *     pnpm -C ui e2e:restricted
 *
 * Skips (nothing is built or started) when FATHOM_RESTRICTED_KUBECONFIG is unset.
 */
const enabled = restrictedKubeconfig() !== undefined

export default defineConfig({
  testDir: './tests/live',
  testMatch: /restricted-walk\.spec\.ts/,
  workers: 1,
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  timeout: 90_000,
  expect: { timeout: 15_000 },
  reporter: 'list',
  globalSetup: './tests/live/global-setup-restricted.ts',
  globalTeardown: './tests/live/global-teardown.ts',
  use: {
    baseURL: APP_ORIGIN,
    trace: 'retain-on-failure',
  },
  projects: [
    {
      name: 'chromium-wide',
      use: {
        ...devices['Desktop Chrome'],
        viewport: { width: 1920, height: 1080 },
        deviceScaleFactor: 1,
      },
    },
  ],
  ...(enabled
    ? {
        webServer: {
          command: LIVE_SERVE_COMMAND,
          url: APP_ORIGIN,
          reuseExistingServer: false,
          timeout: 120_000,
        },
      }
    : {}),
})
