/** ShortCopy and Fingerprint: the ends of a long id, the full value in the title and on Copy. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'short-copy'
const DIGEST = 'sha256:9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08'

test.beforeEach(async ({ context }) => {
  await context.grantPermissions(['clipboard-read', 'clipboard-write'])
})

test('ids: short forms show, and Copy from the keyboard copies the full value', async ({ page }) => {
  await openPiece(page, { piece, state: 'ids' })
  await expect(page.getByText('sha256:9f86d081…b0f00a08')).toHaveAttribute('title', DIGEST)
  await expect(page.getByText('SHA256:uNiVztks…Cvj3tD2s')).toBeVisible()
  await page.keyboard.press('Tab')
  const copy = page.getByRole('button', { name: 'Copy full digest' })
  await expect(copy).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(page.getByRole('status').filter({ hasText: 'Full digest copied' })).toHaveCount(1)
  expect(await page.evaluate(() => navigator.clipboard.readText())).toBe(DIGEST)
})

test('ids: no accessibility violations', async ({ page }) => {
  await expectAccessible(page, { piece, state: 'ids' })
})
