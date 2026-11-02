/** Tabs: a labelled tab list, arrow keys move between tabs, the strip scrolls inside itself. */
import { expect, test } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'tabs'

// The 360 px checks tap, as a phone would.
test.use({ hasTouch: true })

test('a labelled tab list, with data-tab on each tab and one panel', async ({ page }) => {
  await openPiece(page, { piece, state: 'default' })
  const list = page.getByRole('tablist', { name: 'Widget sections' })
  await expect(list.getByRole('tab')).toHaveCount(3)
  const overview = list.getByRole('tab', { name: 'Overview' })
  await expect(overview).toHaveAttribute('data-tab', 'overview')
  await expect(overview).toHaveAttribute('aria-selected', 'true')
  await expect(page.getByRole('tabpanel')).toHaveText('The overview of Blue widget.')
})

test('arrow keys, Home and End move the selection; Tab leaves the strip for the panel', async ({ page }) => {
  await openPiece(page, { piece, state: 'default' })
  await page.keyboard.press('Tab')
  const tab = (name: string) => page.getByRole('tab', { name })
  await expect(tab('Overview')).toBeFocused()
  await page.keyboard.press('ArrowRight')
  await expect(tab('Activity')).toBeFocused()
  await expect(tab('Activity')).toHaveAttribute('aria-selected', 'true')
  await expect(page.getByRole('tabpanel')).toHaveText('The activity of Blue widget.')
  await page.keyboard.press('End')
  await expect(tab('Settings')).toHaveAttribute('aria-selected', 'true')
  await page.keyboard.press('ArrowRight')
  await expect(tab('Overview')).toHaveAttribute('aria-selected', 'true')
  await page.keyboard.press('ArrowLeft')
  await expect(tab('Settings')).toHaveAttribute('aria-selected', 'true')
  await page.keyboard.press('Home')
  await expect(tab('Overview')).toHaveAttribute('aria-selected', 'true')
  await page.keyboard.press('Tab')
  await expect(page.getByRole('tabpanel')).toBeFocused()
})

test('at 360 px many tabs scroll inside the strip, never the page, and a tap selects', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'many' })
  const strip = page.getByRole('tablist').locator('..')
  const [scroll, client] = await strip.evaluate((el) => [el.scrollWidth, el.clientWidth])
  expect(scroll).toBeGreaterThan(client)
  const last = page.getByRole('tab', { name: 'History' })
  await last.scrollIntoViewIfNeeded()
  await last.tap()
  await expect(last).toHaveAttribute('aria-selected', 'true')
  await expect(page.getByRole('tabpanel')).toHaveText('The history of Blue widget.')
})

for (const state of ['default', 'many']) {
  test(`${state} is accessible`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}
