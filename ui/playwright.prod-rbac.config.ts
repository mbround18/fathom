import { defineConfig, devices } from '@playwright/test'

/**
 * Opt-in PROD RBAC suite: sets up (or verifies) the scoped `fathom-e2e/fathom-agent` identity
 * through Fathom's own 011 RBAC UI, against a real (possibly production) cluster. Only
 * `tests/live/rbac-prod-scoped-*.spec.ts` run here, and the bridge runs in strict mode (see
 * tests/live/global-setup-prod-rbac.ts). Plan + record:
 * specs/011-rbac-oidc-administration/checklists/prod-scoped-agent.md.
 *
 *   # as the cluster operator: create the planned bindings (idempotent)
 *   FATHOM_PROD_RBAC_SETUP=1 FATHOM_LIVE_CONTEXT=<ctx> pnpm -C ui e2e:prod-rbac
 *   # as the agent itself (operator-minted kubeconfig): read-only checks of what the UI allows
 *   FATHOM_PROD_RBAC_VERIFY=1 KUBECONFIG=~/.kube/fathom-agent.yaml \
 *     FATHOM_LIVE_CONTEXT=<agent ctx> pnpm -C ui e2e:prod-rbac
 */
const APP_ORIGIN = 'http://localhost:4174'

export default defineConfig({
  testDir: './tests/live',
  testMatch: /rbac-prod-scoped-.*\.spec\.ts/,
  workers: 1,
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  timeout: 180_000,
  expect: { timeout: 20_000 },
  reporter: 'list',
  globalSetup: './tests/live/global-setup-prod-rbac.ts',
  globalTeardown: './tests/live/global-teardown.ts',
  use: {
    baseURL: APP_ORIGIN,
    trace: 'retain-on-failure',
  },
  // One viewport: the setup creates objects, so it must not run once per project.
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
  webServer: {
    command: 'pnpm e2e:live:serve',
    url: APP_ORIGIN,
    reuseExistingServer: false,
    timeout: 120_000,
  },
})
