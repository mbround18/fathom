/**
 * PageHeader `size="pane"` (spec 065 T027): a pane's one-row header, with its lead, title, count
 * and actions, and the pane's own props on the root.
 */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'page-header-pane'

test('a pane header is an h2 naming its pane, with count, actions and root props', async ({ page }) => {
  await openPiece(page, { piece, state: 'default' })
  const pane = page.getByRole('region', { name: 'Pods' })
  await expect(pane.getByRole('heading', { level: 2, name: 'Pods' })).toBeVisible()
  await expect(page.getByRole('heading', { level: 1 })).toHaveCount(0)
  const header = pane.locator('header[data-pane-header]')
  await expect(header).toHaveAttribute('draggable', 'true')
  await expect(header).toHaveAttribute('data-drag-handle', 'pods')
  await expect(header.locator('[data-pane-count]')).toHaveText('12')
  await expect(header.getByText('01')).toBeVisible()

  await page.keyboard.press('Tab')
  await expect(header.getByRole('button', { name: 'Refresh' })).toBeFocused()

  // One row, at least a touch target tall.
  const box = await header.boundingBox()
  expect(box?.height ?? 0).toBeGreaterThanOrEqual(32)
  expect(box?.height ?? 0).toBeLessThan(60)
})

test('a long title truncates and keeps the actions on the row', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'long-title' })
  const header = page.locator('header[data-pane-header]')
  const title = header.getByRole('heading', { level: 2 })
  const button = header.getByRole('button', { name: 'Refresh' })
  await expect(button).toBeInViewport({ ratio: 1 })
  const [t, b] = [await title.boundingBox(), await button.boundingBox()]
  expect(Math.abs((t?.y ?? 0) + (t?.height ?? 0) / 2 - ((b?.y ?? 0) + (b?.height ?? 0) / 2))).toBeLessThan(4)
  expect(await title.evaluate((el) => el.scrollWidth > el.clientWidth)).toBe(true)
})

for (const state of ['default', 'long-title', 'title-only']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}
