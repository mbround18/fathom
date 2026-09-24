/**
 * 032 (B): responsive shell. Sheets fit narrow viewports, tab labels do not clip, tools are
 * reachable without a dock.
 */
import { expect, test, type Page } from '@playwright/test'

import { open } from './helpers'

const cls = (page: Page) => page.locator('html').getAttribute('data-width-class')

test.describe('responsive shell', () => {
  test('phone: scope chip sheet and More sheet fit the viewport', async ({ page }) => {
    await open(page)
    test.skip((await cls(page)) !== 'phone', 'phone only')
    await page.getByTestId('scope-chip').click()
    const scope = page.getByRole('dialog').first()
    await expect(scope).toBeVisible()
    const b = await scope.boundingBox()
    expect((b?.x ?? 0) + (b?.width ?? 0)).toBeLessThanOrEqual((page.viewportSize()?.width ?? 0) + 1)
    await page.keyboard.press('Escape')

    await page.getByRole('button', { name: 'More', exact: true }).click()
    const more = page.getByTestId('more-sheet')
    await expect(more).toBeVisible()
    await expect
      .poll(async () => {
        const m = await more.boundingBox()
        return (
          (m?.x ?? 0) >= -1 &&
          (m?.x ?? 0) + (m?.width ?? 0) <= (page.viewportSize()?.width ?? 0) + 1
        )
      })
      .toBe(true)
  })

  test('phone: bottom bar labels are not clipped', async ({ page }) => {
    await open(page)
    test.skip((await cls(page)) !== 'phone', 'phone only')
    const clipped = await page
      .getByRole('navigation', { name: 'Views' })
      .getByRole('button')
      .evaluateAll((els) => els.filter((e) => e.scrollWidth > e.clientWidth + 1).length)
    expect(clipped).toBe(0)
  })

  test('laptop: Tools menu opens a tool sheet', async ({ page }) => {
    await open(page)
    test.skip((await cls(page)) !== 'laptop', 'laptop only')
    await page.getByTestId('tools-menu').click()
    await page.getByRole('menuitem', { name: /Velero/ }).click()
    await expect(page.getByTestId('tools-sheet')).toBeVisible()
  })

  test('no horizontal page scroll', async ({ page }) => {
    await open(page)
    const over = await page.evaluate(
      () => document.documentElement.scrollWidth - document.documentElement.clientWidth,
    )
    expect(over).toBeLessThanOrEqual(0)
  })
})
