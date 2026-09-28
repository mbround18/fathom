/**
 * 038-sdk-login-ollama: Settings -> AI Assist journeys against the fake backend (the real
 * providers cannot be reached from CI): GitHub device-code sign-in, an invalid API key that is
 * never echoed back, picking an Ollama model, and offline mode.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { clearSent, emit, navigateTo, open, sent } from './helpers'

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

const statuses = (github: 'NotConfigured' | 'Connected', offline = false) => ({
  type: 'AiProviderStatus' as const,
  data: {
    statuses: [
      {
        provider: 'github' as const,
        state: { type: github },
        account_label: github === 'Connected' ? 'octocat' : null,
        last_checked_at: null,
      },
      {
        provider: 'claude' as const,
        state: { type: 'NotConfigured' as const },
        account_label: null,
        last_checked_at: null,
      },
      {
        provider: 'openai' as const,
        state: { type: 'NotConfigured' as const },
        account_label: null,
        last_checked_at: null,
      },
    ],
    active: null,
    offline,
    ollama_model: null,
  },
})

async function goToAi(page: Page) {
  await open(page)
  await navigateTo(page, 'Cluster Settings', 'Team')
  await emit(
    page,
    batch([
      system(1, { type: 'ClusterSettingsLoaded', data: { context: 'kind-a', settings: SETTINGS } }),
    ]),
  )
  // 043 US3: app-wide settings moved from Cluster settings to App settings.
  await navigateTo(page, 'App settings', 'Settings')
  await expect(page.getByTestId('ai-assist-settings')).toBeVisible()
  await emit(page, batch([system(2, statuses('NotConfigured'))]))
}

test.describe('038 AI Assist settings', () => {
  test('GitHub device-code login shows the code and connects', async ({ page }) => {
    await goToAi(page)
    await clearSent(page)
    await page.getByTestId('ai-login-github').click()
    await expect
      .poll(async () => (await sent(page)).map((c) => c.type))
      .toContain('AiProviderLoginStart')

    await emit(
      page,
      batch([
        system(3, {
          type: 'AiLoginFlowUpdate',
          data: {
            flow: {
              provider: 'github',
              kind: 'device_code',
              user_code: 'ABCD-1234',
              verification_uri: 'https://github.com/login/device',
              expires_at: new Date(Date.now() + 600_000).toISOString(),
              interval_s: 5,
              phase: { type: 'Polling' },
            },
          },
        }),
      ]),
    )
    await expect(page.getByTestId('ai-user-code')).toHaveText('ABCD-1234')
    await expect(page.getByTestId('ai-countdown')).toBeVisible()

    await emit(page, batch([system(4, statuses('Connected'))]))
    await expect(page.getByTestId('ai-state-github')).toContainText('Connected')
    await expect(page.getByTestId('ai-signout-github')).toBeVisible()
  })

  test('an invalid API key shows the provider error and clears the field', async ({ page }) => {
    await goToAi(page)
    await page.getByTestId('ai-key-input-claude').fill('sk-ant-bad')
    await page.getByTestId('ai-key-submit-claude').click()
    await expect(page.getByTestId('ai-key-input-claude')).toHaveValue('')
    await emit(
      page,
      batch([
        system(3, {
          type: 'AiError',
          data: {
            code: 'invalid_key',
            message_verbatim: 'Claude rejected the key (401)',
            hint: 'Check the key.',
          },
        }),
      ]),
    )
    await expect(page.getByTestId('ai-error')).toContainText('Claude rejected the key (401)')
    await expect(page.getByTestId('ai-state-claude')).toContainText('Not signed in')
  })

  test('pick an Ollama model', async ({ page }) => {
    await goToAi(page)
    await emit(
      page,
      batch([
        system(3, {
          type: 'AiOllamaState',
          data: {
            selected_model: null,
            state: {
              endpoint: 'http://127.0.0.1:11434',
              reachable: true,
              version: '0.5.1',
              installed_hint: 'found',
              checked_at: new Date().toISOString(),
              error: null,
              models: [
                {
                  name: 'llama3.2:latest',
                  size_bytes: 2e9,
                  family: 'llama',
                  parameter_size: '3.2B',
                  modified_at: '',
                },
                {
                  name: 'qwen2.5:7b',
                  size_bytes: 4.7e9,
                  family: null,
                  parameter_size: null,
                  modified_at: '',
                },
              ],
            },
          },
        }),
      ]),
    )
    await clearSent(page)
    await page.getByTestId('ollama-model-qwen2.5:7b').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'AiOllamaSelectModel'))
      .toEqual({ type: 'AiOllamaSelectModel', data: { model: 'qwen2.5:7b' } })
  })

  test('offline toggle sends the command', async ({ page }) => {
    await goToAi(page)
    await clearSent(page)
    await page.getByTestId('ai-offline-toggle').click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'AiSetOfflineMode'))
      .toEqual({ type: 'AiSetOfflineMode', data: { enabled: true } })
    await emit(page, batch([system(3, statuses('NotConfigured', true))]))
    await expect(page.getByTestId('ai-offline-toggle')).toBeChecked()
  })
})
