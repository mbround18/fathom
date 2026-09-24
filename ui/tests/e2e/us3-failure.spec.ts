/**
 * T091 (US3 scenario 4): a failed action shows an error toast carrying the backend's message
 * verbatim, and the list and selection stay exactly as they were.
 */
import { expect, test } from '@playwright/test'

import { PODS_500 } from '../fixtures'
import {
  actionToast,
  check,
  clearSent,
  confirmDialog,
  expectRowCount,
  finish,
  nextRequestId,
  open,
  row,
  sentTypes,
  pressDelete,
} from './helpers'

const P = PODS_500[0]
if (!P) throw new Error('fixture')
const KEY = `${P.namespace}/${P.name}`
const ERROR = `pods "${P.name}" is forbidden: User "system:serviceaccount:fathom-dev:viewer" cannot delete resource "pods" in API group "" in the namespace "${P.namespace}"`

test('an error result shows the verbatim message and leaves the rows unchanged', async ({
  page,
}) => {
  await open(page)
  await expectRowCount(page, 500)
  await check(page, KEY)
  await clearSent(page)
  await pressDelete(page)
  const dialog = confirmDialog(page)
  await expect(dialog.getByTestId('confirm-item')).toHaveText([`✖${KEY}`])
  const id = await nextRequestId(page)
  await dialog.getByTestId('confirm-submit').click()
  await expect.poll(() => sentTypes(page)).toEqual(['DeletePod'])

  await finish(page, id, false, ERROR)
  const toast = actionToast(page, 'error')
  await expect(toast).toContainText(`Delete failed: pod ${KEY}`)
  await expect(toast).toContainText(ERROR)
  // Status is text and glyph, not colour alone.
  await expect(toast).toContainText('✖')

  await expectRowCount(page, 500)
  await expect(row(page, KEY)).toBeVisible()
  await expect(row(page, KEY).getByRole('checkbox')).toHaveAttribute('aria-checked', 'true')
  expect(await sentTypes(page)).toEqual(['DeletePod'])
})

test('no result within the timeout is reported, not left hanging', async ({ page }) => {
  await page.clock.install()
  await open(page)
  await expectRowCount(page, 500)
  await check(page, KEY)
  await pressDelete(page)
  await confirmDialog(page).getByTestId('confirm-submit').click()
  await expect(page.getByTestId('pending-actions')).toBeVisible()
  await page.clock.runFor(31_000)
  await expect(actionToast(page, 'error')).toContainText('No response from backend')
  await expect(page.getByTestId('pending-actions')).toBeHidden()
  await expect(row(page, KEY)).toBeVisible()
})
