/**
 * 049 US2: ask Flux to reconcile now, optionally fetching the source first. Sources have no
 * "with source" verb; a suspended resource is warned that Flux will ignore the request.
 */
import { expect, test } from '@playwright/test'

import { clearSent, confirmDialog, openedInspector, sent } from './helpers'
import { openFluxRow } from './flux-049'

test.describe('US2 Flux reconcile', () => {
  test('Reconcile with source sends ReconcileFluxResource with with_source: true', async ({
    page,
  }) => {
    await openFluxRow(page, 'HelmRelease fathom-dev/podinfo')
    await openedInspector(page).getByTestId('inspector-actions').click()
    await expect(page.getByTestId('action-reconcile')).toBeVisible()
    await clearSent(page)
    await page.getByTestId('action-reconcile-source').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toContainText('fathom-dev/podinfo')
    await expect(dialog).toContainText('HelmRepository/podinfo')
    expect((await sent(page)).filter((c) => c.type === 'ReconcileFluxResource')).toEqual([])
    await dialog.getByTestId('confirm-submit').click()
    await expect
      .poll(() => sent(page))
      .toEqual([
        {
          type: 'ReconcileFluxResource',
          data: {
            kind: 'HelmRelease',
            namespace: 'fathom-dev',
            name: 'podinfo',
            with_source: true,
          },
        },
      ])
  })

  test('a source has Reconcile but no "with source"; confirming sends with_source: false', async ({
    page,
  }) => {
    await openFluxRow(page, 'GitRepository flux-system/fleet')
    await openedInspector(page).getByTestId('inspector-actions').click()
    await expect(page.getByTestId('action-reconcile')).toBeVisible()
    await expect(page.getByTestId('action-reconcile-source')).toHaveCount(0)
    await clearSent(page)
    await page.getByTestId('action-reconcile').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await expect
      .poll(() => sent(page))
      .toEqual([
        {
          type: 'ReconcileFluxResource',
          data: {
            kind: 'GitRepository',
            namespace: 'flux-system',
            name: 'fleet',
            with_source: false,
          },
        },
      ])
  })

  test('reconciling a suspended resource warns that Flux ignores the request', async ({ page }) => {
    await openFluxRow(page, 'Kustomization flux-system/infra')
    await openedInspector(page).getByTestId('inspector-actions').click()
    await page.getByTestId('action-reconcile').click()
    await expect(confirmDialog(page).getByTestId('flux-suspended-warning')).toBeVisible()
  })
})
