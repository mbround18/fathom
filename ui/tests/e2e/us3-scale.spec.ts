/**
 * T089 (US3 scenario 1): scaling a deployment shows `current → new` before anything is sent,
 * sends exactly one `ScaleDeployment`, and reports the backend's result. Esc sends nothing.
 */
import { expect, test } from '@playwright/test'

import { genDeployments } from '../fixtures/gen-pods'
import {
  actionToast,
  check,
  clearSent,
  DEPLOYMENTS,
  finish,
  isPhone,
  nextRequestId,
  open,
  row,
  scaleDialog,
  sent,
  sentTypes,
  showList,
  table,
} from './helpers'

const D = genDeployments('kind-a').find((d) => d.desired === 1)
if (!D) throw new Error('fixture: a deployment with 1 replica')
const KEY = `${D.namespace}/${D.name}`

test.describe('US3 scale', () => {
  test.beforeEach(async ({ page }) => {
    await open(page)
    await showList(page, DEPLOYMENTS, 'Deployments')
    await expect(row(page, KEY, DEPLOYMENTS)).toBeVisible()
  })

  test('scale to 3 shows 1 → 3 and sends one ScaleDeployment', async ({ page }) => {
    await check(page, KEY, DEPLOYMENTS)
    await page.getByTestId('bulk-scale').click()

    const dialog = scaleDialog(page)
    await expect(dialog).toBeVisible()
    await expect(dialog.getByTestId('scale-target')).toContainText(KEY)
    await expect(dialog.getByTestId('scale-current')).toHaveText('1')
    const input = dialog.getByLabel('Replicas', { exact: true })
    await expect(input).toBeFocused()
    await input.fill('3')
    await expect(dialog.getByTestId('scale-preview')).toContainText('1')
    await expect(dialog.getByTestId('scale-new')).toHaveText('3')
    await expect(dialog.getByTestId('scale-submit')).toHaveText('Scale 1 → 3')

    await clearSent(page)
    const id = await nextRequestId(page)
    await dialog.getByTestId('scale-submit').click()
    await expect(dialog).toBeHidden()
    await expect
      .poll(() => sent(page))
      .toEqual([
        { type: 'ScaleDeployment', data: { namespace: D.namespace, name: D.name, replicas: 3 } },
      ])
    await expect(page.getByTestId('pending-actions')).toHaveAttribute(
      'data-request-ids',
      String(id),
    )

    // Nothing changes until the backend answers.
    await expect(row(page, KEY, DEPLOYMENTS)).toBeVisible()
    await finish(page, id, true)
    await expect(actionToast(page, 'success')).toContainText(`Scaled deployment ${KEY} to 3`)
    await expect(page.getByTestId('pending-actions')).toBeHidden()
    await expect.poll(() => sentTypes(page)).toContain('RefreshDeployments')
    expect((await sentTypes(page)).filter((t) => t === 'ScaleDeployment')).toHaveLength(1)
  })

  test('Esc and Cancel send nothing; focus returns to the list', async ({ page }) => {
    await check(page, KEY, DEPLOYMENTS)
    await clearSent(page)
    await page.getByTestId('bulk-scale').click()
    await scaleDialog(page).getByLabel('Replicas', { exact: true }).fill('4')
    await page.keyboard.press('Escape')
    await expect(scaleDialog(page)).toBeHidden()
    await expect(table(page, DEPLOYMENTS).getByRole('grid')).toBeFocused()

    await page.getByTestId('bulk-scale').click()
    await scaleDialog(page).getByTestId('scale-cancel').click()
    await expect(scaleDialog(page)).toBeHidden()
    expect(await sent(page)).toEqual([])
  })

  test('reachable from the row menu and the pane-header menu', async ({ page }, info) => {
    const r = row(page, KEY, DEPLOYMENTS)
    await r.click({ button: 'right' })
    const menu = page.getByTestId('row-action-menu')
    await expect(menu).toBeVisible()
    await expect(menu).toContainText(KEY)
    await menu.getByTestId('action-scale').click()
    await expect(scaleDialog(page)).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(scaleDialog(page)).toBeHidden()

    // The list header menu acts on the checked rows.
    await check(page, KEY, DEPLOYMENTS)
    const trigger = page
      .locator(`[data-pane-kind="list"][data-section="${DEPLOYMENTS}"]`)
      .first()
      .getByTestId('list-actions')
    await trigger.click()
    await page.getByTestId('action-scale').click()
    await expect(scaleDialog(page)).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(trigger).toBeFocused()

    if (!isPhone(info)) {
      const box = await trigger.boundingBox()
      expect(box?.height).toBeGreaterThanOrEqual(24)
    } else {
      const box = await trigger.boundingBox()
      expect(box?.height).toBeGreaterThanOrEqual(44)
      expect(box?.width).toBeGreaterThanOrEqual(44)
    }
  })
})
