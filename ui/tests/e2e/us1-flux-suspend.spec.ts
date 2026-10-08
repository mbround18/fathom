/**
 * 049 US1: suspend and resume a Flux resource from its inspector. Nothing leaves the UI until the
 * dialog that names the object (and what it reaches) is confirmed; only the applicable verb is
 * offered.
 */
import { expect, test } from '@playwright/test'

import { clearSent, confirmDialog, openedInspector, sent } from './helpers'
import { openFluxRow } from './flux-049'

test.describe('US1 Flux suspend and resume', () => {
  test('Suspend names the Kustomization and its inventory, then sends SuspendFluxResource', async ({
    page,
  }) => {
    await openFluxRow(page, 'Kustomization flux-system/apps')
    await openedInspector(page).getByTestId('inspector-actions').click()
    await expect(page.getByTestId('action-suspend')).toBeVisible()
    await expect(page.getByTestId('action-resume')).toHaveCount(0)
    await clearSent(page)
    await page.getByTestId('action-suspend').click()
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await expect(dialog).toContainText('flux-system/apps')
    await expect(dialog.getByTestId('flux-reach')).toContainText('Deployment default/web')
    await expect(dialog.getByTestId('flux-reach')).toContainText('ConfigMap default/web-env')
    expect((await sent(page)).filter((c) => c.type === 'SuspendFluxResource')).toEqual([])
    await dialog.getByTestId('confirm-submit').click()
    await expect
      .poll(() => sent(page))
      .toEqual([
        {
          type: 'SuspendFluxResource',
          data: { kind: 'Kustomization', namespace: 'flux-system', name: 'apps', suspend: true },
        },
      ])
  })

  test('a suspended resource offers Resume, not Suspend, and sends suspend: false', async ({
    page,
  }) => {
    await openFluxRow(page, 'Kustomization flux-system/infra')
    await openedInspector(page).getByTestId('inspector-actions').click()
    await expect(page.getByTestId('action-resume')).toBeVisible()
    await expect(page.getByTestId('action-suspend')).toHaveCount(0)
    await clearSent(page)
    await page.getByTestId('action-resume').click()
    await confirmDialog(page).getByTestId('confirm-submit').click()
    await expect
      .poll(() => sent(page))
      .toEqual([
        {
          type: 'SuspendFluxResource',
          data: { kind: 'Kustomization', namespace: 'flux-system', name: 'infra', suspend: false },
        },
      ])
  })

  test('suspending a source names the Kustomizations that pull from it', async ({ page }) => {
    await openFluxRow(page, 'GitRepository flux-system/fleet')
    await openedInspector(page).getByTestId('inspector-actions').click()
    await page.getByTestId('action-suspend').click()
    const reach = confirmDialog(page).getByTestId('flux-reach')
    await expect(reach).toContainText('Kustomization flux-system/apps')
    await expect(reach).toContainText('Kustomization flux-system/infra')
    await page.keyboard.press('Escape')
    await expect(confirmDialog(page)).toHaveCount(0)
    expect((await sent(page)).filter((c) => c.type === 'SuspendFluxResource')).toEqual([])
  })
})
