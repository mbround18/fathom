/**
 * Navigator reconciliation: Events and every CRD-backed integration must be reachable from the
 * Sections nav on every width class, open their pane, send their refresh, and (when the CRDs are
 * absent) stay clickable but muted with a "not installed" hint rather than an error.
 */
import { expect, test, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { emit, navigateTo, open, sentTypes } from './helpers'

const pane = (page: Page, section: string) =>
  page.locator(`section[data-slot="pane"][data-section="${section}"]`)

test.describe('nav reaches Events and integrations', () => {
  test('Events is a normal (not muted) entry and opens its pane', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Events', 'Overview')
    await expect(pane(page, 'events')).toBeVisible()
    await expect.poll(() => sentTypes(page)).toContain('RefreshEvents')
  })

  for (const [label, section, cmd] of [
    ['Flux', 'integrations/flux', 'RefreshFlux'],
    ['Velero', 'integrations/velero', 'RefreshVelero'],
    ['Knative', 'integrations/knative', 'RefreshKnative'],
    ['KEDA Autoscalers', 'integrations/keda', 'RefreshKeda'],
    ['Certificates', 'integrations/cert-manager', 'RefreshCertManager'],
    ['Hubble', 'integrations/hubble', 'RefreshHubble'],
  ] as const) {
    test(`${label} opens from Integrations and refreshes`, async ({ page }) => {
      await open(page)
      await navigateTo(page, label, 'Integrations')
      await expect(pane(page, section)).toBeVisible()
      await expect.poll(() => sentTypes(page)).toContain(cmd)
    })
  }

  test('a not-installed integration is muted with a hint yet still opens', async ({ page }) => {
    await open(page)
    await navigateTo(page, 'Flux', 'Integrations')
    await emit(
      page,
      batch([
        system(1, {
          type: 'FluxOverviewUpdated',
          data: { installed: false, sources: [], kustomizations: [], helmreleases: [] },
        }),
      ]),
    )
    await expect(pane(page, 'integrations/flux')).toContainText('CRDs were not found')
    await navigateTo(page, 'Events', 'Overview')
    await navigateTo(page, 'Flux', 'Integrations')
    await expect(pane(page, 'integrations/flux')).toBeVisible()
    const nav = page.getByRole('navigation', { name: 'Sections' })
    const drawer = !(await nav.isVisible())
    if (drawer) await page.getByRole('button', { name: 'Open sections' }).click()
    const entry = nav.locator('[data-section="integrations/flux"]')
    await expect(entry).toHaveAttribute('data-installed', 'false')
    await expect(entry).toHaveAttribute('title', 'Not installed in this cluster')
  })
})
