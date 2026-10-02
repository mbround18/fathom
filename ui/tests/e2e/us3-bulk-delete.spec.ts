/**
 * T090 (US3 scenarios 2–3): bulk delete names every `namespace/name` before sending; Cancel
 * sends nothing, confirm sends one `BulkDelete`, more than five needs the count typed, and the
 * list changes only when the backend's refreshed list arrives.
 */
import { expect, test } from '@playwright/test'

import { PODS_500, pods as podsBatch } from '../fixtures'
import { genDeployments } from '../fixtures/gen-pods'
import {
  actionToast,
  check,
  clearSent,
  confirmDialog,
  DEPLOYMENTS,
  emit,
  expectRowCount,
  finish,
  nextRequestId,
  open,
  row,
  sent,
  sentTypes,
  showList,
  table,
  pressDelete,
} from './helpers'

const key = (p: { namespace: string; name: string }) => `${p.namespace}/${p.name}`
const pair = (p: { namespace: string; name: string }): [string, string] => [p.namespace, p.name]
const THREE = PODS_500.slice(0, 3)
const SIX = PODS_500.slice(0, 6)

test.describe('US3 bulk delete', () => {
  test.beforeEach(async ({ page }) => {
    await open(page)
    await expectRowCount(page, 500)
  })

  test('Delete key on 3 selected pods lists each; Cancel sends nothing; confirm sends BulkDelete', async ({
    page,
  }) => {
    for (const p of THREE) await check(page, key(p))
    await expect(page.getByTestId('selection-count')).toContainText('3 selected')
    await clearSent(page)

    await pressDelete(page)
    const dialog = confirmDialog(page)
    await expect(dialog).toBeVisible()
    await expect(dialog.getByTestId('confirm-item')).toHaveText(THREE.map((p) => `✖${key(p)}`))
    // Any bulk delete makes the user state the count (`TYPED_CONFIRM_THRESHOLD`), so the
    // destructive button is never armed on first paint here.
    await expect(dialog.getByTestId('confirm-submit')).toBeDisabled()
    await dialog.getByTestId('confirm-cancel').click()
    await expect(dialog).toBeHidden()
    await expect(table(page).getByRole('grid')).toBeFocused()
    expect(await sent(page)).toEqual([])

    await pressDelete(page)
    await expect(dialog).toBeVisible()
    await page.keyboard.press('Escape')
    await expect(dialog).toBeHidden()
    expect(await sent(page)).toEqual([])

    await page.getByTestId('bulk-delete').click()
    const id = await nextRequestId(page)
    await dialog.getByTestId('confirm-typed-count').fill('3')
    await dialog.getByTestId('confirm-submit').click()
    await expect(dialog).toBeHidden()
    await expect
      .poll(() => sent(page))
      .toEqual([{ type: 'BulkDelete', data: { items: THREE.map(pair) } }])

    // No optimistic removal: rows stay until the backend's refreshed list.
    await expectRowCount(page, 500)
    await finish(page, id, true)
    await expect(actionToast(page, 'success')).toContainText('Deleted 3 pods')
    await expect.poll(() => sentTypes(page)).toContain('RefreshPods')
    await expect(page.getByTestId('selection-bar')).toBeHidden()
    // Let the fake's own refresh reply land, then deliver the post-delete list.
    await page.waitForTimeout(50)
    await emit(page, podsBatch('kind-a', PODS_500.slice(3), 31))
    await expectRowCount(page, 497)
    for (const p of THREE) await expect(row(page, key(p))).toHaveCount(0)
  })

  test('more than 5 needs the count typed', async ({ page }) => {
    for (const p of SIX) await check(page, key(p))
    await clearSent(page)
    await page.getByTestId('bulk-delete').click()
    const dialog = confirmDialog(page)
    await expect(dialog.getByTestId('confirm-item')).toHaveCount(6)
    const submit = dialog.getByTestId('confirm-submit')
    await expect(submit).toBeDisabled()
    const typed = dialog.getByLabel('Type 6 to confirm')
    await typed.fill('5')
    await expect(submit).toBeDisabled()
    await typed.fill('6')
    await expect(submit).toBeEnabled()
    await typed.press('Enter')
    await expect(dialog).toBeHidden()
    await expect
      .poll(() => sent(page))
      .toEqual([{ type: 'BulkDelete', data: { items: SIX.map(pair) } }])
  })

  test('deployments: bulk delete sends one DeleteDeployment per row', async ({ page }) => {
    await showList(page, DEPLOYMENTS, 'Deployments')
    const two = genDeployments('kind-a').slice(0, 2)
    for (const d of two) await check(page, key(d), DEPLOYMENTS)
    await clearSent(page)
    await page.getByTestId('bulk-delete').click()
    const dialog = confirmDialog(page)
    await expect(dialog.getByTestId('confirm-item')).toHaveText(two.map((d) => `✖${key(d)}`))
    await dialog.getByTestId('confirm-typed-count').fill('2')
    await dialog.getByTestId('confirm-submit').click()
    await expect
      .poll(() => sent(page))
      .toEqual(
        two.map((d) => ({
          type: 'DeleteDeployment',
          data: { namespace: d.namespace, name: d.name },
        })),
      )
  })
})
