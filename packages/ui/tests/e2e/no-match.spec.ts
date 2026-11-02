/** NoMatch: a list a filter emptied says so, and Clear filter brings the rows back. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'no-match'

test('says nothing matches, and Clear filter from the keyboard clears it', async ({ page }) => {
  await openPiece(page, { piece, state: 'default' })
  const empty = page.locator('[data-pane-state="empty"]')
  await expect(empty).toHaveAttribute('data-empty', 'no-match')
  await expect(empty).toContainText('No widgets match.')

  await page.getByRole('textbox', { name: 'Filter widgets' }).focus()
  await page.keyboard.press('Tab')
  await expect(page.getByRole('button', { name: 'Clear filter' })).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(page.getByRole('textbox', { name: 'Filter widgets' })).toHaveValue('')
  await expect(page.getByRole('list', { name: 'Widgets' }).getByRole('listitem')).toHaveCount(3)
  await expect(empty).toHaveCount(0)
})

test('a caller’s message replaces the default, and a tap clears at 360 px', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'message' })
  await expect(page.locator('[data-empty="no-match"]')).toContainText('No widgets match “purple”.')
  await page.getByRole('button', { name: 'Clear filter' }).click()
  await expect(page.getByRole('list', { name: 'Widgets' })).toBeVisible()
})

for (const state of ['default', 'message']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}
