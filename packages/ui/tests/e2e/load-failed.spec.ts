/** LoadFailed: "Failed to load {what}." with the reason, and Retry when the caller can retry. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'load-failed'

test('says what failed and why, as an error pane, and Retry runs the load again', async ({ page }) => {
  await openPiece(page, { piece, state: 'retry' })
  await expect(page.locator('[data-pane-state="error"]')).toHaveCount(1)
  await expect(page.getByRole('alert')).toHaveText('Failed to load widgets. The widget store did not answer in time. (unavailable)')

  const retry = page.getByRole('button', { name: 'Retry' })
  await retry.focus()
  await page.keyboard.press('Enter')
  await expect(page.locator('[data-retries]')).toHaveText('Retried 1 time')
  await page.keyboard.press('Space')
  await expect(page.locator('[data-retries]')).toHaveText('Retried 2 times')
})

test('Retry is a comfortable tap at 360 px', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'retry' })
  await page.getByRole('button', { name: 'Retry' }).click()
  await expect(page.locator('[data-retries]')).toHaveText('Retried 1 time')
})

test('without onRetry there is no Retry', async ({ page }) => {
  await openPiece(page, { piece, state: 'no-retry' })
  await expect(page.getByRole('alert')).toHaveText('Failed to load widgets. Denied: list_widgets (no_grant)')
  await expect(page.getByRole('button')).toHaveCount(0)
})

for (const state of ['retry', 'no-retry']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}
