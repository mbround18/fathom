/**
 * LIVE (041-observability-autoconnect) against kind `fathom-dev`, which runs Prometheus
 * (`fathom-metrics-prometheus/prometheus:9090`) and Grafana (`grafana/grafana:3000`):
 * - Topology renders the active namespace's graph on phone, laptop and wide.
 * - Metrics auto-connects to the detected Prometheus through the API server proxy, no input.
 * - Open Grafana starts a Fathom port-forward (listed under Port forwards) that answers; the spec
 *   stops and removes the forward when it finishes.
 * Screenshots go to `$SHOT_DIR` when set.
 */
import type { Page } from '@playwright/test'

import { navigateTo } from '../e2e/helpers'

import { expect, test } from './helpers'

const OUT = process.env.SHOT_DIR
const SIZES = {
  phone: { width: 390, height: 844 },
  laptop: { width: 1280, height: 800 },
  wide: { width: 1920, height: 1080 },
}

async function shot(page: Page, name: string): Promise<void> {
  if (OUT) await page.screenshot({ path: `${OUT}/${name}.png` })
}

test.describe.configure({ mode: 'serial' })

test('Topology shows the namespace graph at every width', async ({ page, app }) => {
  test.skip(test.info().project.name !== 'chromium-wide', 'viewport sizes are set explicitly')
  test.setTimeout(180_000)
  for (const [name, size] of Object.entries(SIZES)) {
    await page.setViewportSize(size)
    await app.open()
    const nav = page.getByRole('navigation', { name: 'Sections' })
    if (!(await nav.isVisible())) await page.getByRole('button', { name: 'Open sections' }).click()
    await nav.locator('[data-section="topology"]').click()
    const summary = page.getByTestId('topology-summary').first()
    // Namespaces arrive first; wait for the workload and pod tiers (edges) too.
    await expect(summary).toHaveText(/^[1-9]\d* nodes, [1-9]\d* edges/, { timeout: 30_000 })
    const box = await page.getByTestId('topology-canvas').first().boundingBox()
    expect(box?.width ?? 0).toBeGreaterThan(size.width >= 1280 ? 600 : 300)
    await page.waitForTimeout(1500) // let the layout settle before the screenshot
    await shot(page, `topology-${name}`)
    await page.keyboard.press('Escape')
  }
})

test('Metrics auto-connects to the detected Prometheus', async ({ page, app }) => {
  test.setTimeout(120_000)
  await app.open()
  await navigateTo(page, 'Metrics', 'Integrations')
  await expect(page.getByTestId('prometheus-test-result')).toContainText(
    'fathom-metrics-prometheus/prometheus:9090 via the API server proxy',
    { timeout: 60_000 },
  )
  await expect(page.getByTestId('metrics-status')).toHaveText('Prometheus: connected')
  // A real query through the proxy, read-only: kind's static etcd pod has a stable name.
  await page.getByLabel('Pod namespace').fill('kube-system')
  await page.getByLabel('Pod name', { exact: true }).fill('etcd-fathom-dev-control-plane')
  await page.getByRole('button', { name: 'Fetch once' }).click()
  await expect(page.getByTestId('pod-metrics-row').first()).toContainText(
    'kube-system/etcd-fathom-dev-control-plane',
    { timeout: 30_000 },
  )
  await shot(page, 'metrics')
})

test('Open Grafana forwards the Service and it answers', async ({ page, app, context }) => {
  test.setTimeout(120_000)
  await app.open()
  await navigateTo(page, 'Grafana', 'Integrations')
  await page.evaluate(() => {
    window.open = () => null
  })
  const item = page.getByTestId('grafana-detected-item').filter({ hasText: 'grafana/grafana:3000' })
  await expect(item).toBeVisible({ timeout: 60_000 })
  await item.getByTestId('grafana-connect').click()
  const link = item.getByTestId('grafana-local-url')
  await expect(link).toHaveText(/^http:\/\/localhost:\d+$/, { timeout: 60_000 })
  const url = (await link.textContent()) ?? ''
  const res = await context.request.get(`${url}/api/health`)
  expect(res.ok()).toBe(true)
  await shot(page, 'grafana')

  await navigateTo(page, 'Port Forwards', 'Network')
  const row = page.getByTestId('port-forward-row').filter({ hasText: 'grafana' }).first()
  await expect(row).toBeVisible()
  await shot(page, 'grafana-port-forwards')
  await row.getByTestId('port-forward-stop').click()
  await page.getByRole('button', { name: 'Stop forward' }).click()
  await row.getByTestId('port-forward-delete').click()
  await page.getByRole('button', { name: 'Remove forward' }).click()
  await expect(page.getByTestId('port-forward-row').filter({ hasText: 'grafana' })).toHaveCount(0)
})
