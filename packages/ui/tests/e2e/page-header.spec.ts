/** PageHeader: the h1, an optional intro, "Learn more" with a guide, actions wrapping on a phone. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'page-header'

test('page: the h1 carries the id, and the actions follow it in Tab order', async ({ page }) => {
  await openPiece(page, { piece, state: 'page' })
  const h1 = page.getByRole('heading', { level: 1, name: 'Widgets' })
  await expect(h1).toHaveAttribute('id', 'widgets-title')
  await page.keyboard.press('Tab')
  await expect(page.getByRole('button', { name: 'Export' })).toBeFocused()
  await page.keyboard.press('Tab')
  await expect(page.getByRole('button', { name: 'Add widget' })).toBeFocused()
})

test('intro: a plain description, with no guide link', async ({ page }) => {
  await openPiece(page, { piece, state: 'intro' })
  await expect(page.getByText('Everything the harness lists, newest first.')).toBeVisible()
  await expect(page.locator('[data-page-intro]')).toHaveCount(0)
  await expect(page.getByRole('link')).toHaveCount(0)
})

test('guide: one clamped line on desktop, then Learn more', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 })
  await openPiece(page, { piece, state: 'guide' })
  const intro = page.locator('[data-page-intro]')
  const learn = intro.getByRole('link', { name: 'Learn more' })
  await expect(learn).toHaveAttribute('href', '/help/widgets')
  await expect(learn).toHaveAttribute('data-guide-link', 'widgets')
  const clamp = await intro.locator('p').evaluate((el) => getComputedStyle(el).webkitLineClamp)
  expect(clamp).toBe('1')
  await expect(page.getByRole('combobox', { name: 'Show' })).toBeVisible()
})

test('at 360 px the actions wrap under the title and the description is not clamped', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'guide' })
  const h1 = await page.getByRole('heading', { level: 1 }).boundingBox()
  const add = await page.getByRole('button', { name: 'Add widget' }).boundingBox()
  expect(add?.y ?? 0).toBeGreaterThan((h1?.y ?? 0) + (h1?.height ?? 0) - 1)
  const clamp = await page.locator('[data-page-intro] p').evaluate((el) => getComputedStyle(el).webkitLineClamp)
  expect(clamp).toBe('none')
})

for (const state of ['page', 'intro', 'guide']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}
