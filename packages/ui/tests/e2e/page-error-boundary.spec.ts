/** PageErrorBoundary: a page that throws says so, with the message and a Reload. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'page-error-boundary'

test('a thrown page shows the error as an error pane, and Reload reloads', async ({ page }) => {
  await openPiece(page, { piece, state: 'thrown' })
  await expect(page.locator('[data-pane-state="error"]')).toHaveCount(1)
  await expect(page.getByRole('alert')).toHaveText('Something went wrong showing this page. widgets is undefined')
  await expect(page.locator('[data-error-message]')).toHaveText('widgets is undefined')

  // The harness flag is unset by a real reload, and set again once the page settles.
  await page.evaluate(() => {
    window.__kitReady = false
    ;(window as { __beforeReload?: boolean }).__beforeReload = true
  })
  await page.keyboard.press('Tab')
  await expect(page.getByRole('button', { name: 'Reload' })).toBeFocused()
  await page.keyboard.press('Enter')
  await page.waitForFunction(() => window.__kitReady === true)
  expect(await page.evaluate(() => (window as { __beforeReload?: boolean }).__beforeReload)).toBeUndefined()
  await expect(page.locator('[data-error-message]')).toHaveText('widgets is undefined')
})

test('a page that renders shows its children only', async ({ page }) => {
  await openPiece(page, { piece, state: 'healthy' })
  await expect(page.getByText('Widgets rendered without trouble.')).toBeVisible()
  await expect(page.getByRole('alert')).toHaveCount(0)
  await expect(page.locator('[data-pane-state]')).toHaveCount(0)
})

for (const state of ['thrown', 'healthy']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}
