/** NavShell: grouped section links, a badge and a dot, a sidebar on desktop and a strip on a phone. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'nav-shell'

// The 360 px checks tap, as a phone would.
test.use({ hasTouch: true })

test('groups, the current page, the badge and the dot are marked and read out', async ({ page }) => {
  await openPiece(page, { piece, state: 'default' })
  const nav = page.getByRole('navigation', { name: 'Sections' })
  const work = nav.getByRole('group', { name: 'Work' })
  await expect(work).toHaveAttribute('data-nav-group', 'work')
  await expect(nav.getByRole('group', { name: 'People' })).toHaveAttribute('data-nav-group', 'people')

  const widgets = work.getByRole('link', { name: 'Widgets (3 widgets waiting for review)' })
  await expect(widgets).toHaveAttribute('aria-current', 'page')
  await expect(nav.locator('[data-nav-badge="widgets"]')).toHaveText('3')
  // A count of 0 is hidden.
  await expect(nav.locator('[data-nav-badge="gadgets"]')).toHaveCount(0)
  await expect(work.getByRole('link', { name: 'Gadgets', exact: true })).not.toHaveAttribute('aria-current')

  const settings = nav.getByRole('link', { name: 'Settings (update available)' })
  await expect(settings.locator('[data-update-dot]')).toHaveCount(1)
  await expect(page.getByRole('heading', { level: 1, name: 'Widgets' })).toBeVisible()
})

test('from the keyboard, Enter on a section moves aria-current and the page', async ({ page }) => {
  await openPiece(page, { piece, state: 'default' })
  await page.keyboard.press('Tab')
  await expect(page.getByRole('link', { name: /^Widgets/ })).toBeFocused()
  await page.keyboard.press('Tab')
  const gadgets = page.getByRole('link', { name: 'Gadgets', exact: true })
  await expect(gadgets).toBeFocused()
  await page.keyboard.press('Enter')
  await expect(gadgets).toHaveAttribute('aria-current', 'page')
  await expect(page.getByRole('link', { name: /^Widgets/ })).not.toHaveAttribute('aria-current')
  await expect(page.getByRole('heading', { level: 1, name: 'Gadgets' })).toBeVisible()
  await expect(page).toHaveURL(/\/gadgets$/)
})

test('on desktop: a column with visible group headings and Settings pinned to the bottom', async ({ page }) => {
  await page.setViewportSize({ width: 1280, height: 800 })
  await openPiece(page, { piece, state: 'default' })
  const nav = page.getByRole('navigation', { name: 'Sections' })
  const navBox = await nav.boundingBox()
  const settings = await nav.getByRole('link', { name: /^Settings/ }).boundingBox()
  const history = await nav.getByRole('link', { name: 'History' }).boundingBox()
  if (!navBox || !settings || !history) throw new Error('no layout')
  expect(navBox.width).toBeLessThan(260)
  expect(settings.y).toBeGreaterThan(history.y + history.height)
  expect(navBox.y + navBox.height - (settings.y + settings.height)).toBeLessThan(16)
  await expect(nav.getByText('Work', { exact: true })).toBeVisible()
})

test('at 360 px: a strip that scrolls inside itself, and a tap on the pinned item selects it', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'default' })
  const nav = page.getByRole('navigation', { name: 'Sections' })
  const [scroll, client] = await nav.evaluate((el) => [el.scrollWidth, el.clientWidth])
  expect(scroll).toBeGreaterThan(client)
  const settings = nav.getByRole('link', { name: /^Settings/ })
  await settings.scrollIntoViewIfNeeded()
  await settings.tap()
  await expect(settings).toHaveAttribute('aria-current', 'page')
  await expect(page.getByRole('heading', { level: 1, name: 'Settings' })).toBeVisible()
})

test('banner: a notice sits under the header, above the frame', async ({ page }) => {
  await openPiece(page, { piece, state: 'banner' })
  const banner = page.getByRole('status')
  await expect(banner).toHaveText('Reconnecting… changes are saved when the connection is back.')
  const header = await page.getByText('Widget desk').boundingBox()
  const notice = await banner.boundingBox()
  const nav = await page.getByRole('navigation', { name: 'Sections' }).boundingBox()
  if (!header || !notice || !nav) throw new Error('no layout')
  expect(notice.y).toBeGreaterThan(header.y)
  expect(nav.y).toBeGreaterThanOrEqual(notice.y + notice.height - 1)
})

for (const state of ['default', 'banner']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}
