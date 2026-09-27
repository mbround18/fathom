/**
 * Smoke: every pane added by the recent feature specs opens on desktop and renders without console
 * errors or uncaught page exceptions. Deep behaviour lives in each pane's own spec; this guards
 * against a merge leaving a pane unmountable (missing store slice, bad registration, throw on mount).
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { emit, navigateTo, open, openFleet, sentTypes } from './helpers'

// Same shape as the 030 kind spec's loaded settings.
const BASE_SETTINGS = {
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

test.describe('new panes smoke (desktop)', () => {
  let errors: string[] = []

  test.beforeEach(({ page }, info) => {
    test.skip(
      !info.project.name.endsWith('-desktop'),
      'dock tabs and pane layout are desktop-only here',
    )
    errors = []
    page.on('pageerror', (e) => errors.push(`pageerror: ${e.message}`))
    page.on('console', (m) => {
      if (m.type() === 'error') errors.push(`console.error: ${m.text()}`)
    })
  })

  test.afterEach(() => {
    expect(errors, 'console/page errors while rendering the pane').toEqual([])
  })

  const dockTab = async (page: Page, name: string, testId?: string) => {
    await open(page)
    const tab = page.getByRole('tab', { name })
    await tab.click()
    await expect(tab).toHaveAttribute('aria-selected', 'true')
    if (testId) await expect(page.getByTestId(testId).first()).toBeVisible()
  }

  test('Knative', ({ page }) => dockTab(page, 'Knative'))
  test('Keda autoscalers', ({ page }) => dockTab(page, 'Autoscalers'))
  test('Linter', ({ page }) => dockTab(page, 'Linter', 'linter-panel'))
  test('Velero', ({ page }) => dockTab(page, 'Velero'))
  test('Certificates', ({ page }) => dockTab(page, 'Certificates'))

  test('Trivy vulnerabilities', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Vulnerabilities', 'Integrations')
    await expect(page.getByRole('region', { name: 'Vulnerabilities' })).toBeVisible()
  })

  test('Gatekeeper', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Gatekeeper', 'Integrations')
    await expect(page.getByRole('region', { name: 'Gatekeeper' })).toBeVisible()
  })

  test('Fleet dashboard', async ({ page }) => {
    await open(page, 'manual')
    await emit(
      page,
      batch([
        system(1, {
          type: 'ContextsUpdated',
          data: { contexts: ['prod', 'dev'], current: 'prod' },
        }),
      ]),
    )
    await openFleet(page)
    await expect(page.getByTestId('fleet-dashboard')).toBeVisible()
  })

  test('Audit log', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Audit Log', 'Team')
    await expect(page.getByTestId('audit-log')).toBeVisible()
  })

  test('Local Clusters (kind) panel', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Cluster Settings', 'Team')
    await expect.poll(() => sentTypes(page)).toContain('LoadClusterSettings')
    await emit(
      page,
      batch([
        system(1, {
          type: 'ClusterSettingsLoaded',
          data: { context: 'kind-a', settings: BASE_SETTINGS },
        }),
      ]),
    )
    // 043 US3: kind provisioning moved from Cluster settings to Local clusters.
    await navigateTo(page, 'Local clusters', 'Settings')
    await expect(page.getByTestId('kind-clusters')).toBeVisible()
  })

  test('Command palette', async ({ page }) => {
    await open(page)
    await page.keyboard.press('Control+k')
    await expect(page.getByRole('dialog')).toBeVisible()
    await expect(page.locator('[cmdk-input]')).toBeVisible()
  })
})
