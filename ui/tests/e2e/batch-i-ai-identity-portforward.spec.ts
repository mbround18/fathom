/** Batch I: Ollama endpoint picker + file-store opt-in, OIDC role-map editor, port-forward delete. */
import { expect, test, type Page } from '@playwright/test'

import { batch, globalRoute, list, system } from '../fixtures/builders'
import type { PortForward } from '../../src/protocol/types'
import { clearSent, emit, navigateTo, open, sent, sentTypes } from './helpers'

const SETTINGS = {
  schema: 1 as const,
  metricsSource: 'none' as const,
  prometheusPath: '/api/v1',
  https: false,
  pathPrefix: '',
  hiddenMetrics: [],
  orgRevKnown: null,
  fieldOrigin: {
    metricsSource: 'local' as const,
    prometheusPath: 'local' as const,
    https: 'local' as const,
    pathPrefix: 'local' as const,
    hiddenMetrics: 'local' as const,
  },
}

async function goToAi(page: Page) {
  await open(page)
  await navigateTo(page, 'Cluster Settings', 'Team')
  await emit(
    page,
    batch([
      system(1, { type: 'ClusterSettingsLoaded', data: { context: 'kind-a', settings: SETTINGS } }),
    ]),
  )
  await expect(page.getByTestId('ai-assist-settings')).toBeVisible()
}

const ollama = (over: Record<string, unknown>) =>
  system(3, {
    type: 'AiOllamaState',
    data: {
      selected_model: null,
      state: {
        endpoint: 'http://127.0.0.1:11434',
        reachable: false,
        version: null,
        installed_hint: 'found',
        checked_at: new Date().toISOString(),
        error: 'connection refused',
        models: [],
        ...over,
      },
    },
  } as never)

test.describe('Ollama endpoint picker', () => {
  test('unreachable state, then test and save a LAN endpoint', async ({ page }) => {
    await goToAi(page)
    await emit(page, batch([ollama({})]))
    await expect(page.getByTestId('ollama-not-running')).toContainText('ollama serve')
    await expect(page.getByTestId('ollama-endpoint-test')).toBeDisabled()
    await expect(page.getByTestId('ollama-endpoint-save')).toBeDisabled()

    await page.getByTestId('ollama-endpoint-input').fill('http://192.168.1.5:11434')
    await clearSent(page)
    await page.getByTestId('ollama-endpoint-test').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'AiOllamaProbe'))
      .toEqual({ type: 'AiOllamaProbe', data: { endpoint: 'http://192.168.1.5:11434' } })

    await page.getByTestId('ollama-allow-lan').check()
    await page.getByTestId('ollama-endpoint-save').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'AiSetOllamaEndpoint'))
      .toEqual({
        type: 'AiSetOllamaEndpoint',
        data: { endpoint: 'http://192.168.1.5:11434', allow_lan: true },
      })
  })

  test('Re-check probes the default endpoint', async ({ page }) => {
    await goToAi(page)
    await clearSent(page)
    await page.getByTestId('ollama-recheck').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'AiOllamaProbe'))
      .toEqual({ type: 'AiOllamaProbe', data: { endpoint: null } })
  })

  test('file-store opt-in needs the acknowledgement checkbox', async ({ page }) => {
    await goToAi(page)
    await expect(page.getByTestId('ai-file-store')).toHaveCount(0)
    const provider = (p: string, state: unknown) => ({
      provider: p,
      state,
      account_label: null,
      last_checked_at: null,
    })
    await emit(
      page,
      batch([
        system(2, {
          type: 'AiProviderStatus',
          data: {
            statuses: [
              provider('github', { type: 'Blocked', data: { reason: 'store_unavailable' } }),
              provider('claude', { type: 'NotConfigured' }),
              provider('openai', { type: 'NotConfigured' }),
            ],
            active: null,
            offline: false,
            ollama_model: null,
          },
        } as never),
      ]),
    )
    await expect(page.getByTestId('ai-file-store')).toBeVisible()
    await expect(page.getByTestId('ai-file-store-enable')).toBeDisabled()
    await clearSent(page)
    await page.getByTestId('ai-file-store-confirm').check()
    await page.getByTestId('ai-file-store-enable').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'AiEnableFileStore'))
      .toEqual({ type: 'AiEnableFileStore', data: { confirm: true } })
  })
})

const issuer = {
  id: 'rauthy',
  issuer_url: 'https://auth.example.com/auth/v1',
  client_id: 'fathom',
  scopes: ['openid'],
  ca_bundle_path: null,
  username_claim: 'preferred_username',
  groups_claim: 'groups',
  username_prefix: 'oidc:',
  groups_prefix: 'oidc:',
  admin_api_base: null,
}

async function openRoleMap(page: Page, role_map: object, groups: string[]) {
  await open(page)
  await emit(
    page,
    batch([
      system(10, {
        type: 'OidcConfigLoaded',
        data: {
          config: { issuers: [issuer], contexts: [], role_map },
          identities: [
            {
              issuer_id: 'rauthy',
              sub: 'u1',
              username: 'ada',
              email: null,
              groups,
              expires_at: 4102444800,
              state: { type: 'SignedIn' },
            },
          ],
        },
      } as never),
    ]),
  )
  await page.getByTestId('identity-chip').click()
  await expect(page.getByTestId('oidc-role-map')).toBeVisible()
}

test.describe('OIDC role-map editor', () => {
  test('bootstrap: editable with no admin group, saves trimmed values', async ({ page }) => {
    await openRoleMap(page, {}, [])
    await expect(page.getByTestId('oidc-role-map-save')).toBeDisabled()
    await page.getByTestId('oidc-role-viewers_group').fill('  oidc:viewers ')
    await page.getByTestId('oidc-role-admins_group').fill('oidc:admins')
    await clearSent(page)
    await page.getByTestId('oidc-role-map-save').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'OidcSetRoleMap'))
      .toEqual({
        type: 'OidcSetRoleMap',
        data: {
          role_map: { viewers_group: 'oidc:viewers', editors_group: '', admins_group: 'oidc:admins' },
        },
      })
  })

  test('read-only for a non-admin once an admin group exists', async ({ page }) => {
    await openRoleMap(
      page,
      { viewers_group: 'v', editors_group: 'e', admins_group: 'oidc:admins' },
      ['oidc:viewers'],
    )
    await expect(page.getByTestId('oidc-role-map-readonly')).toBeVisible()
    await expect(page.getByTestId('oidc-role-admins_group')).toBeDisabled()
    await expect(page.getByTestId('oidc-role-map-save')).toBeDisabled()
  })

  test('editable for a member of the admin group', async ({ page }) => {
    await openRoleMap(
      page,
      { viewers_group: 'v', editors_group: 'e', admins_group: 'oidc:admins' },
      ['oidc:admins'],
    )
    await expect(page.getByTestId('oidc-role-map-readonly')).toHaveCount(0)
    await page.getByTestId('oidc-role-editors_group').fill('ed2')
    await expect(page.getByTestId('oidc-role-map-save')).toBeEnabled()
  })
})

test.describe('port-forward delete', () => {
  const pf = (over: Partial<PortForward>): PortForward => ({
    id: 'pf-1',
    namespace: 'default',
    pod: 'web-1',
    local_port: 8080,
    remote_port: 8080,
    container: 'web',
    status: 'Closed',
    auto_start: false,
    ...over,
  })

  test('Remove is offered only for Closed/Error rows and sends DeletePortForward', async ({
    page,
  }) => {
    await open(page)
    await navigateTo(page, 'Port Forwards', 'Network')
    await expect.poll(() => sentTypes(page)).toContain('RefreshPortForwards')
    await emit(
      page,
      batch([
        list(1, globalRoute('network/portforwards'), {
          type: 'PortForwardsUpdated',
          data: [pf({}), pf({ id: 'pf-2', pod: 'db-1', status: 'Active' })],
        }),
      ]),
    )
    const closed = page.locator('[data-testid="port-forward-row"][data-tunnel-id="pf-1"]')
    const active = page.locator('[data-testid="port-forward-row"][data-tunnel-id="pf-2"]')
    await expect(active.getByTestId('port-forward-delete')).toHaveCount(0)
    await clearSent(page)
    await closed.getByTestId('port-forward-delete').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'DeletePortForward'))
      .toEqual({ type: 'DeletePortForward', data: { tunnel_id: 'pf-1' } })
  })
})
