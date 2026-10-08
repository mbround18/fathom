/**
 * 049 US3: every Flux resource is reachable from the pane. Rows are grouped by kind with the ones
 * needing attention first, and a click opens the resource's inspector (which asks the backend for
 * its details) with the verbs in its menu.
 */
import { expect, test } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import { emit, navigateTo, open, openedInspector, sent, sentTypes } from './helpers'
import { FLUX } from './flux-049'

test.describe('US3 Flux pane reaches every resource', () => {
  test('lists all resources, not-ready first per kind, and opens one on click', async ({
    page,
  }) => {
    await open(page)
    await navigateTo(page, 'Flux', 'Integrations')
    await expect.poll(() => sentTypes(page)).toContain('RefreshFlux')
    await emit(page, batch([system(1, { type: 'FluxOverviewUpdated', data: FLUX })]))
    const pane = page.locator('[data-slot="pane"][data-section="integrations/flux"]').first()
    const rows = pane.getByTestId('flux-row')
    await expect(rows).toHaveCount(5)
    await expect(rows.nth(0)).toContainText('GitRepository flux-system/fleet')
    await expect(rows.nth(1)).toContainText('HelmRepository fathom-dev/podinfo')
    // Within the Kustomization group the suspended one comes before the healthy one.
    await expect(rows.nth(2)).toContainText('Kustomization flux-system/infra')
    await expect(rows.nth(2)).toContainText('Suspended')
    await expect(rows.nth(3)).toContainText('Kustomization flux-system/apps')
    await expect(rows.nth(4)).toContainText('HelmRelease fathom-dev/podinfo')

    await rows.nth(4).click()
    await expect
      .poll(() => sent(page))
      .toEqual(
        expect.arrayContaining([
          {
            type: 'SelectFluxResource',
            data: { kind: 'HelmRelease', namespace: 'fathom-dev', name: 'podinfo' },
          },
        ]),
      )
    const insp = openedInspector(page)
    await expect(insp).toContainText('podinfo')
    await insp.getByTestId('inspector-actions').click()
    for (const id of ['action-suspend', 'action-reconcile', 'action-reconcile-source']) {
      await expect(page.getByTestId(id)).toBeVisible()
    }
  })
})
