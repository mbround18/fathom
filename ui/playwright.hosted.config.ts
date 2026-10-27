import { defineConfig, devices } from '@playwright/test'

/**
 * Opt-in LIVE hosted-mode suite (spec 031, T038): the real `fathom-server` image behind
 * ingress-nginx on a kind cluster whose API server trusts a real OIDC provider (Rauthy or
 * Keycloak). Bring the cluster up first, then run:
 *
 *   PROVIDER=rauthy k8s/cluster.sh up-hosted
 *   PROVIDER=rauthy [CLUSTER=...] [FATHOM_HOSTED_IP=<node ip>] pnpm -C ui e2e:hosted
 *
 * `FATHOM_HOSTED_IP` is needed when the cluster was created without host ports 80/443 (another
 * hosted cluster owns them): Chromium then maps *.localtest.me to the kind node's docker IP and
 * the provider admin calls in helpers.ts do the same. Sign-in goes through the provider's real
 * login page (auth-code + PKCE); the throwaway CA is accepted via `ignoreHTTPSErrors`.
 *
 * One worker, no retries: the specs share the provider's users and the cluster's `demo`
 * namespace, and the revocation spec disables and re-enables a user.
 */
const ip = process.env.FATHOM_HOSTED_IP
// HOSTED_BROWSER=webkit runs the same specs in WebKit (Safari cookie/SameSite behaviour).
// WebKit has no resolver switch, so with FATHOM_HOSTED_IP it goes through a local CONNECT
// proxy (tests/hosted/resolve-proxy.mjs) that tunnels *.localtest.me to the kind node.
const webkit = process.env.HOSTED_BROWSER === 'webkit'
const proxyPort = Number(process.env.RESOLVE_PROXY_PORT ?? 18443)
const provider = process.env.PROVIDER ?? 'rauthy'

export default defineConfig({
  testDir: './tests/hosted',
  workers: 1,
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: 0,
  timeout: 120_000,
  expect: { timeout: 20_000 },
  reporter: 'list',
  use: {
    baseURL: 'https://fathom.localtest.me',
    ignoreHTTPSErrors: true,
    trace: 'retain-on-failure',
    ...(webkit ? devices['Desktop Safari'] : devices['Desktop Chrome']),
    viewport: { width: 1600, height: 900 },
    ...(webkit
      ? ip
        ? { proxy: { server: `http://127.0.0.1:${proxyPort}` } }
        : {}
      : ip
        ? { launchOptions: { args: [`--host-resolver-rules=MAP *.localtest.me ${ip}`] } }
        : {}),
  },
  ...(webkit && ip
    ? {
        webServer: {
          command: 'node tests/hosted/resolve-proxy.mjs',
          port: proxyPort,
          reuseExistingServer: false,
        },
      }
    : {}),
  projects: [{ name: `${webkit ? 'webkit' : 'chromium'}-${provider}` }],
})
