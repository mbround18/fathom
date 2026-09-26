/** 039: Grafana and Prometheus settings persist and hydrate; the key never reaches the UI. */
import { expect, test } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import {
  chooseScope,
  clearSent,
  emit,
  expectNoHorizontalScroll,
  expectScope,
  navigateTo,
  open,
  sent,
} from './helpers'

const KEY = 'sup3r-secret-key'

let seq = 500
const grafana = (context: string, base_url: string | null, key_set = false) =>
  batch([
    system(seq++, {
      type: 'ObservabilitySettings',
      data: {
        context,
        prometheus: null,
        grafana: base_url === null ? null : { base_url, key_set },
      },
    }),
  ])

test.describe('039 observability settings', () => {
  test('Grafana save, reopen, key shown set, clear key; key never in DOM or frames', async ({
    page,
  }) => {
    await open(page)
    await navigateTo(page, 'Grafana', 'Integrations')
    await expect(page.getByTestId('grafana-configured')).toContainText('Not configured')
    await expect
      .poll(async () => (await sent(page)).some((c) => c.type === 'GetObservabilitySettings'))
      .toBe(true)

    await clearSent(page)
    await page.getByLabel('Base URL').fill('https://g.example')
    await page.getByLabel('API key').fill(KEY)
    await page.getByRole('button', { name: 'Save' }).click()
    const cmds = await sent(page)
    expect(cmds.find((c) => c.type === 'ConfigureGrafana')).toEqual({
      type: 'ConfigureGrafana',
      data: { base_url: 'https://g.example', api_key: KEY },
    })
    await emit(page, grafana('kind-a', 'https://g.example', true))
    await expect(page.getByTestId('grafana-key-state')).toHaveText('Key set')
    await expect(page.getByLabel('API key')).toHaveValue('')
    expect(await page.content()).not.toContain(KEY)

    // Reopen: URL prefilled, key input empty, blank-key save keeps the key.
    await navigateTo(page, 'Metrics', 'Integrations')
    await navigateTo(page, 'Grafana', 'Integrations')
    await emit(page, grafana('kind-a', 'https://g.example', true))
    await expect(page.getByLabel('Base URL')).toHaveValue('https://g.example')
    await expect(page.getByLabel('API key')).toHaveValue('')
    await clearSent(page)
    await page.getByRole('button', { name: 'Save' }).click()
    const resave = await sent(page)
    expect(resave.find((c) => c.type === 'ConfigureGrafana')).toEqual({
      type: 'ConfigureGrafana',
      data: { base_url: 'https://g.example', api_key: '' },
    })
    // No frame sent after the first save carries the key text.
    expect(JSON.stringify(resave)).not.toContain(KEY)

    await page.getByRole('button', { name: 'Clear key' }).click()
    expect((await sent(page)).some((c) => c.type === 'ClearGrafanaKey')).toBe(true)
    await emit(page, grafana('kind-a', 'https://g.example', false))
    await expect(page.getByTestId('grafana-key-state')).toHaveText('Key not set')
    await expect(page.getByRole('button', { name: 'Clear key' })).toBeDisabled()
    await expectNoHorizontalScroll(page)
  })

  test('context switch isolates settings and ignores late replies', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Grafana', 'Integrations')
    await emit(page, grafana('kind-a', 'https://a.example', true))
    await expect(page.getByLabel('Base URL')).toHaveValue('https://a.example')

    await clearSent(page)
    await chooseScope(page, 'Context', 'kind-b')
    await expectScope(page, 'Context', 'kind-b')
    await expect
      .poll(async () => (await sent(page)).some((c) => c.type === 'GetObservabilitySettings'))
      .toBe(true)
    // A late reply for the previous context is ignored.
    await emit(page, grafana('kind-a', 'https://late.example', true))
    await expect(page.getByLabel('Base URL')).not.toHaveValue('https://late.example')
    await emit(page, grafana('kind-b', 'https://b.example', false))
    await expect(page.getByLabel('Base URL')).toHaveValue('https://b.example')
    await expect(page.getByTestId('grafana-key-state')).toHaveText('Key not set')
    await expectNoHorizontalScroll(page)
  })

  test('keyring failure notice is shown and the key stays unset', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Grafana', 'Integrations')
    await emit(
      page,
      batch([
        system(seq++, {
          type: 'Notification',
          data: { level: 'Error', message: 'Credential store unavailable' },
        }),
      ]),
    )
    await emit(page, grafana('kind-a', 'https://g.example', false))
    await expect(page.getByTestId('status-notice')).toContainText('Credential store unavailable')
    await expect(page.getByTestId('grafana-key-state')).toHaveText('Key not set')
    await expect(page.getByLabel('Base URL')).toHaveValue('https://g.example')
  })

  test('Prometheus prefill, edit, re-save', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Metrics', 'Integrations')
    const form = page.getByRole('region', { name: 'Prometheus connection' })
    await expect(page.getByTestId('prometheus-configured')).toContainText('Not configured')
    await emit(
      page,
      batch([
        system(seq++, {
          type: 'ObservabilitySettings',
          data: {
            context: 'kind-a',
            prometheus: { namespace: 'obs', service: 'prom', port: 9091 },
            grafana: null,
          },
        }),
      ]),
    )
    await expect(form.getByLabel('Namespace')).toHaveValue('obs')
    await expect(form.getByLabel('Port')).toHaveValue('9091')
    await expect(page.getByTestId('prometheus-configured')).toHaveText('Configured')
    await form.getByLabel('Service').fill('prom2')
    await clearSent(page)
    await form.getByRole('button', { name: 'Save' }).click()
    expect((await sent(page)).find((c) => c.type === 'ConfigurePrometheus')).toEqual({
      type: 'ConfigurePrometheus',
      data: { namespace: 'obs', service: 'prom2', port: 9091 },
    })
    await expectNoHorizontalScroll(page)
  })
})
