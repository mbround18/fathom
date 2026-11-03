/** DetailList: terms and values in a grid; one column on a phone, two pairs side by side wider. */
import { expect, test, type Locator } from '@playwright/test'

import { expectAccessible, openPiece } from '../support/a11y'

const piece = 'detail-list'

async function box(l: Locator) {
  const b = await l.boundingBox()
  if (!b) throw new Error('not laid out')
  return b
}

test('single: each value sits beside its term, and a long one wraps at 360 px', async ({ page }) => {
  await page.setViewportSize({ width: 360, height: 740 })
  await openPiece(page, { piece, state: 'single' })
  await expect(page.locator('dl[aria-label="Widget"]')).toBeVisible()
  await expect(page.locator('[data-field="name"]')).toHaveText('blue-widget')
  await expect(page.locator('[data-field="owner"]')).toHaveText('unknown')
  const term = await box(page.getByText('Name', { exact: true }))
  const value = await box(page.locator('[data-field="name"]'))
  expect(value.x).toBeGreaterThan(term.x + term.width)
  expect(Math.abs(value.y - term.y)).toBeLessThan(2)
  await expect(page.locator('time').filter({ hasText: '5 minutes ago' })).toHaveAttribute('title', '2026-11-02 11:55 UTC')
  const [scroll, client] = await page.evaluate(() => [document.documentElement.scrollWidth, document.documentElement.clientWidth])
  expect(scroll).toBeLessThanOrEqual(client)
})

test('columns: two pairs share a row at 1280 px and stack at 360 px', async ({ page }) => {
  const top = async (text: string) => (await box(page.getByText(text, { exact: true }))).y
  await page.setViewportSize({ width: 1280, height: 800 })
  await openPiece(page, { piece, state: 'columns' })
  expect(Math.abs((await top('Subject')) - (await top('Issuer')))).toBeLessThan(2)
  await page.setViewportSize({ width: 360, height: 740 })
  expect(await top('Issuer')).toBeGreaterThan(await top('Subject'))
})

for (const state of ['single', 'columns']) {
  test(`${state}: no accessibility violations`, async ({ page }) => {
    await expectAccessible(page, { piece, state })
  })
}
