/** ErrorLine: one announced error, dismissable when the caller allows it. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'error-line'

test('is an alert with the message, and Dismiss from the keyboard removes it', async ({ page }) => {
  await openPiece(page, { piece, state: 'dismiss' })
  await expect(page.getByRole('alert')).toHaveText('The widget name is already taken.Dismiss')
  await page.keyboard.press('Tab')
  await expect(page.getByRole('button', { name: 'Dismiss' })).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(page.getByRole('alert')).toHaveCount(0)
  await expect(page.locator('[data-dismissed]')).toBeVisible()
})

test('Dismiss works with a tap at 360 px', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'dismiss' })
  await page.getByRole('button', { name: 'Dismiss' }).click()
  await expect(page.getByRole('alert')).toHaveCount(0)
})

test('without onDismiss there is nothing to press', async ({ page }) => {
  await openPiece(page, { piece, state: 'plain' })
  await expect(page.getByRole('alert')).toHaveText('The widget name is already taken.')
  await expect(page.getByRole('button')).toHaveCount(0)
})

test('keeps the message’s line breaks', async ({ page }) => {
  await openPiece(page, { piece, state: 'multiline' })
  const lines = await page.getByRole('alert').evaluate((el) => (el as HTMLElement).innerText.split('\n').length)
  expect(lines).toBe(3)
})

for (const state of ['dismiss', 'plain', 'multiline']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}
