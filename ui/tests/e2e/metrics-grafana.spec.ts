import { expect, test } from '@playwright/test'

import { batch, globalRoute, list, system } from '../fixtures/builders'
import { clearSent, emit, finish, navigateTo, open, requestIdOf, sent } from './helpers'

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

  test('Open Grafana forwards the detected Service through Fathom and opens it', async ({
    page,
  }) => {
    await open(page)
    await navigateTo(page, 'Grafana', 'Integrations')
    await page.evaluate(() => {
      const w = window as unknown as { __opened: string[] }
      w.__opened = []
      window.open = (url?: string | URL) => {
        w.__opened.push(String(url))
        return null
      }
    })
    await emit(
      page,
      batch([
        system(1, {
          type: 'MetricsStackScanned',
          data: {
            tier: 'Partial',
            components: [{ kind: 'Grafana', namespace: 'grafana', service: 'grafana', port: 3000 }],
            suggested_endpoint: null,
          },
        } as never),
      ]),
    )
    await clearSent(page)
    await page.getByTestId('grafana-connect').click()
    const id = await requestIdOf(page, 'StartServicePortForward')
    const cmd = (await sent(page)).find((c) => c.type === 'StartServicePortForward')
    expect(cmd?.data).toEqual({ namespace: 'grafana', service: 'grafana', port: 3000 })
    await emit(
      page,
      batch([
        list(2, globalRoute('network/portforwards'), {
          type: 'PortForwardsUpdated',
          data: [
            {
              id: 'pf-7',
              namespace: 'grafana',
              pod: 'grafana-abc',
              local_port: 3001,
              remote_port: 3000,
              container: null,
              status: 'Active',
              auto_start: false,
            },
          ],
        }),
        system(3, {
          type: 'ServicePortForwardReady',
          data: {
            namespace: 'grafana',
            service: 'grafana',
            port: 3000,
            local_port: 3001,
            tunnel_id: 'pf-7',
          },
        }),
      ]),
    )
    await finish(page, id, true)
    await expect(page.getByTestId('grafana-local-url')).toHaveText('http://localhost:3001')
    await expect(page.getByLabel('Base URL')).toHaveValue('http://localhost:3001')
    await expect
      .poll(async () => (await sent(page)).find((c) => c.type === 'ConfigureGrafana')?.data)
      .toEqual({ base_url: 'http://localhost:3001', api_key: '' })
    expect(
      await page.evaluate(() => (window as unknown as { __opened: string[] }).__opened),
    ).toEqual(['http://localhost:3001'])
    // The tunnel is an ordinary Fathom forward, tagged with its Service.
    await navigateTo(page, 'Port Forwards', 'Network')
    await expect(page.getByTestId('port-forward-row')).toContainText('grafana')
  })

  test('Open Grafana shows a backend failure', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Grafana', 'Integrations')
    await emit(
      page,
      batch([
        system(1, {
          type: 'MetricsStackScanned',
          data: {
            tier: 'Partial',
            components: [{ kind: 'Grafana', namespace: 'grafana', service: 'grafana', port: 3000 }],
            suggested_endpoint: null,
          },
        } as never),
      ]),
    )
    await page.getByTestId('grafana-connect').click()
    const id = await requestIdOf(page, 'StartServicePortForward')
    await finish(page, id, false, 'No running pod behind Service grafana/grafana')
    await expect(page.getByTestId('grafana-connect-error')).toContainText('No running pod')
  })
})
