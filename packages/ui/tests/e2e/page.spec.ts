/** Page: a named region labelled by its title, with the page's sections inside. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'page'

test('is a region named by its h1, marked data-page', async ({ page }) => {
  await openPiece(page, { piece, state: 'default' })
  const region = page.getByRole('region', { name: 'Widgets' })
  await expect(region).toHaveAttribute('data-page', 'widgets')
  await expect(region.getByRole('heading', { level: 1, name: 'Widgets' })).toBeVisible()
  await expect(region.getByRole('region', { name: 'Recent' })).toBeVisible()
})

test('keeps one gap between its sections at both widths', async ({ page }) => {
  for (const width of [360, 1280]) {
    await page.setViewportSize({ width, height: 800 })
    await openPiece(page, { piece, state: 'default' })
    const gap = await page.locator('[data-page]').evaluate((el) => getComputedStyle(el).rowGap)
    expect(gap).toBe('24px')
  }
})

test('default is accessible', async ({ page }) => {
  await expectAccessible(page, { piece, state: 'default' })
})
