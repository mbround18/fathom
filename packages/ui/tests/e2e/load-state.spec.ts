/** LoadState: a quiet status while loading, LoadFailed when failed, the children once loaded. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'load-state'

test('loading is a status line, not the content', async ({ page }) => {
  await openPiece(page, { piece, state: 'loading' })
  const status = page.getByRole('status')
  await expect(status).toHaveText('Loading widgets…')
  await expect(status).toHaveAttribute('data-pane-state', 'loading')
  await expect(page.getByRole('list', { name: 'Widgets' })).toHaveCount(0)
})

test('failed shows LoadFailed, and Retry from the keyboard hands over to the content', async ({ page }) => {
  await openPiece(page, { piece, state: 'failed' })
  await expect(page.locator('[data-pane-state="error"]')).toHaveCount(1)
  await expect(page.getByRole('alert')).toContainText('Failed to load widgets.')
  await page.keyboard.press('Tab')
  await expect(page.getByRole('button', { name: 'Retry' })).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(page.getByRole('list', { name: 'Widgets' })).toBeVisible()
  await expect(page.locator('[data-pane-state]')).toHaveCount(0)
})

test('loaded renders only the children', async ({ page }) => {
  await openPiece(page, { piece, state: 'loaded' })
  await expect(page.getByRole('list', { name: 'Widgets' }).getByRole('listitem')).toHaveCount(2)
  await expect(page.getByRole('status')).toHaveCount(0)
  await expect(page.getByRole('alert')).toHaveCount(0)
})

for (const state of ['loading', 'failed', 'loaded']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}
