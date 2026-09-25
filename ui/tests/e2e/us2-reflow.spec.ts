/** US2 (T080): search, selection and scroll survive a resize to phone and back. */
import { expect, test } from '@playwright/test'

import { listScroll, visibleRowKey, widthClass } from './layout-helpers'
import { expectRowCount, inspector, listPane, open, row, searchBox, table } from './helpers'

test.describe('US2 reflow', () => {
  test('search text, selection and scroll are preserved across wide -> phone -> wide', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1920, height: 1080 })
    await open(page)
    test.skip((await widthClass(page)) !== 'wide', 'starts at wide')
    await expectRowCount(page, 500)

    await searchBox(page).fill('a')
    await expect
      .poll(async () => Number(await table(page).getAttribute('data-row-count')))
      .toBeGreaterThan(0)
    const filtered = (await table(page).getAttribute('data-row-count')) ?? ''
    await expect.poll(() => listScroll(page, 300)).toBeGreaterThan(100)
    const key = await visibleRowKey(page)
    await row(page, key).click()
    await expect(inspector(page)).toBeVisible()
    await expect(row(page, key)).toHaveAttribute('aria-selected', 'true')
    await page.waitForTimeout(200)
    const scrolled = await listScroll(page)

    await page.setViewportSize({ width: 390, height: 844 })
    await expect.poll(() => widthClass(page)).toBe('phone')
    await page.setViewportSize({ width: 1920, height: 1080 })
    await expect.poll(() => widthClass(page)).toBe('wide')

    await expect(searchBox(page)).toHaveValue('a')
    await expect(table(page)).toHaveAttribute('data-row-count', filtered)
    await expect(row(page, key)).toHaveAttribute('aria-selected', 'true')
    await expect.poll(() => listScroll(page)).toBeGreaterThanOrEqual(scrolled - 5)
    await expect(listPane(page)).toBeVisible()
  })

  test('a dialog with half-typed input survives a resize', async ({ page }) => {
    await page.setViewportSize({ width: 1920, height: 1080 })
    await open(page)
    test.skip((await widthClass(page)) !== 'wide', 'starts at wide')
    await page.keyboard.press('Control+k')
    const input = page.getByRole('combobox').first()
    await expect(input).toBeVisible()
    await input.fill('half typed')
    await page.setViewportSize({ width: 390, height: 844 })
    await expect.poll(() => widthClass(page)).toBe('phone')
    await expect(page.getByRole('dialog')).toBeVisible()
    await expect(page.getByRole('combobox').first()).toHaveValue('half typed')
  })
})
