/**
 * US5 (T109): keyboard-only operation. Arrow/Enter on a focused list, a visible focus ring, and
 * the shortcut registry (`?` overlay, `/` filter focus, `g p` navigation). The spec's axe-core
 * scan is not used: the project asserts its a11y contracts directly (a11y-shell.spec.ts).
 */
import { expect, test } from '@playwright/test'

import { DEPLOYMENTS, expectRowCount, inspector, open, table } from './helpers'
import { widthClass } from './layout-helpers'

test.describe('US5 keyboard', () => {
  test('ArrowDown then Enter on a focused list opens the next row in the inspector', async ({
    page,
  }) => {
    await open(page)
    await expectRowCount(page, 500)
    const grid = table(page).getByRole('grid')
    await grid.focus()
    await page.keyboard.press('ArrowDown')
    await page.keyboard.press('ArrowDown')
    const active = await grid.getAttribute('aria-activedescendant')
    expect(active).toBeTruthy()
    await page.keyboard.press('Enter')
    await expect(inspector(page)).toBeVisible()
    await page.keyboard.press('Escape')
  })

  test('the focus ring is visible on the list and on toolbar buttons', async ({ page }) => {
    await open(page)
    await expectRowCount(page, 500)
    await table(page).getByRole('grid').focus()
    const outline = async () =>
      page.evaluate(() => {
        const s = getComputedStyle(document.activeElement as Element)
        return `${s.outlineStyle}|${s.outlineWidth}|${s.boxShadow}`
      })
    expect(await outline()).not.toMatch(/^none\|0px\|none$/)
    await page.keyboard.press('Shift+Tab')
    expect(await outline()).not.toMatch(/^none\|0px\|none$/)
  })

  test('Tab moves focus through the shell without trapping it', async ({ page }) => {
    await open(page)
    const seen = new Set<string>()
    for (let i = 0; i < 40; i++) {
      await page.keyboard.press('Tab')
      const pane = await page.evaluate(() => {
        const el = document.activeElement
        return el?.closest('[data-slot="pane"]')?.getAttribute('data-pane-kind') ?? 'chrome'
      })
      seen.add(pane)
    }
    expect(seen.size).toBeGreaterThan(1)
  })

  test('? opens the shortcut list and Esc closes it', async ({ page }) => {
    await open(page)
    await page.locator('body').press('?')
    const overlay = page.getByTestId('shortcuts-overlay')
    await expect(overlay).toBeVisible()
    await expect(overlay).toContainText('Focus the filter of the active list')
    await expect(overlay).toContainText('Go to Pods')
    await page.keyboard.press('Escape')
    await expect(overlay).toBeHidden()
  })

  test('/ focuses the list filter and g d goes to Deployments', async ({ page }) => {
    await open(page)
    test.skip((await widthClass(page)) === 'phone', 'phone shows the filter after nav; skip')
    await expectRowCount(page, 500)
    await page.locator('body').press('/')
    await expect(page.locator('[data-pane-kind="list"] input[type="search"]').first()).toBeFocused()
    await page.keyboard.press('Escape')
    await page.locator('body').click({ position: { x: 1, y: 1 } })
    await page.keyboard.press('g')
    await page.keyboard.press('d')
    await expect(
      page.locator(`[data-testid="resource-table"][data-section="${DEPLOYMENTS}"]`).first(),
    ).toBeVisible()
  })
})
