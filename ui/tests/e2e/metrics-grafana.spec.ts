import { expect, test } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { clearSent, emit, navigateTo, open, sent } from './helpers'

test.describe('Metrics and Grafana panes', () => {
  test('Prometheus test result and stack scan render', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Metrics', 'Integrations')
    await clearSent(page)
    await page.getByRole('button', { name: 'Test connection' }).click()
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'TestPrometheusConnection'))
      .toBeTruthy()
    await emit(
      page,
      batch([
        system(1, { type: 'PrometheusConnectionTest', data: { success: true, message: 'ok' } }),
        system(2, {
          type: 'MetricsStackScanned',
          data: {
            tier: 'Full',
            components: [
              { kind: 'Prometheus', namespace: 'monitoring', service: 'prom', port: 9090 },
            ],
            suggested_endpoint: null,
          },
        } as never),
      ]),
    )
    await expect(page.getByTestId('prometheus-test-result')).toContainText('ok')
    await expect(page.getByTestId('stack-scan')).toContainText('monitoring/prom:9090')
  })

  test('Grafana test result shows without echoing the key', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Grafana', 'Integrations')
    await page.getByLabel('Base URL').fill('https://g.example')
    await page.getByLabel('API key').fill('sekret')
    await page.getByRole('button', { name: 'Test connection' }).click()
    await emit(
      page,
      batch([
        system(1, { type: 'GrafanaConnectionTest', data: { success: false, message: 'denied' } }),
      ]),
    )
    await expect(page.getByTestId('grafana-test-result')).toContainText('denied')
    await expect(page.getByText('sekret')).toHaveCount(0)
  })
})
