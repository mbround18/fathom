/** EmptyState: guided steps with the one action, who can act instead, or a plain line. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'empty-state'

test('guided: reason, numbered steps, the action and Learn more', async ({ page }) => {
  await openPiece(page, { piece, state: 'guided' })
  const root = page.locator('[data-pane-state="empty"]')
  await expect(root).toHaveAttribute('data-empty', 'guided')
  await expect(root).toHaveAttribute('data-empty-page', 'widgets')
  await expect(root.getByText('No widgets yet', { exact: true })).toBeVisible()
  await expect(root.getByRole('listitem')).toHaveCount(3)
  await expect(root.locator('[data-empty-action]').getByRole('button', { name: 'Add widget' })).toBeVisible()
  await expect(root.locator('[data-who-can]')).toHaveCount(0)

  const learn = root.getByRole('link', { name: 'Learn more: No widgets yet' })
  await expect(learn).toHaveAttribute('href', '/help/widgets#add-a-widget')
  await expect(learn).toHaveAttribute('data-guide-link', 'widgets')

  // Tab order: the action first, then the guide.
  await page.keyboard.press('Tab')
  await expect(page.getByRole('button', { name: 'Add widget' })).toBeFocused()
  await page.keyboard.press('Tab')
  await expect(learn).toBeFocused()
})

test('restricted: who can act, and never a button', async ({ page }) => {
  await openPiece(page, { piece, state: 'restricted' })
  const root = page.locator('[data-pane-state="empty"]')
  await expect(root).toHaveAttribute('data-empty', 'restricted')
  await expect(root.locator('[data-who-can]')).toHaveText('An owner or admin can add widgets.')
  await expect(root.getByRole('button')).toHaveCount(0)
  await expect(root.locator('[data-empty-action]')).toHaveCount(0)
})

test('plain: no page, so no data-empty', async ({ page }) => {
  await openPiece(page, { piece, state: 'plain' })
  const root = page.locator('[data-pane-state="empty"]')
  await expect(root).not.toHaveAttribute('data-empty')
  await expect(root).not.toHaveAttribute('data-empty-page')
  await expect(root).toContainText('Widgets you add show up here.')
  await expect(root.getByRole('link')).toHaveCount(0)
})

test('page: inside a page under its header, and a tap on the action works at 360 px', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'page' })
  const section = page.getByRole('region', { name: 'Widgets' })
  await expect(section).toHaveAttribute('data-page', 'widgets')
  await expect(section.locator('[data-empty="guided"]')).toBeVisible()
  const box = await page.getByRole('button', { name: 'Add widget' }).boundingBox()
  expect(box?.height ?? 0).toBeGreaterThanOrEqual(24)
})

for (const state of ['guided', 'plain', 'restricted', 'page']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}
