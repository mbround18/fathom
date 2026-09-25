/** US2 (T079): ultra shows five or more slots plus the dock, and more than wide does. */
import { expect, test } from '@playwright/test'

import { slotPanes, widthClass } from './layout-helpers'
import { expectNoHorizontalScroll, open } from './helpers'

test.describe('US2 ultra', () => {
  test('at least 5 slot panes are visible on load, plus the dock', async ({ page }) => {
    await open(page)
    test.skip((await widthClass(page)) !== 'ultra', 'ultra only')
    expect(await slotPanes(page).count()).toBeGreaterThanOrEqual(5)
    const kinds = await slotPanes(page).evaluateAll((els) =>
      els.map((e) => e.getAttribute('data-pane-kind')),
    )
    expect(kinds).toEqual(expect.arrayContaining(['nav', 'list', 'inspector', 'tool']))
    expect(kinds.filter((k) => k === 'list')).toHaveLength(2)
    await expect(page.getByTestId('dock-tabs')).toBeVisible()
    await expectNoHorizontalScroll(page)
  })

  test('more slots show than at wide', async ({ page }) => {
    await open(page)
    test.skip((await widthClass(page)) !== 'ultra', 'ultra only')
    const ultra = await slotPanes(page).count()
    await page.setViewportSize({ width: 1920, height: 1080 })
    await expect.poll(() => widthClass(page)).toBe('wide')
    await expect.poll(() => slotPanes(page).count()).toBeLessThan(ultra)
  })
})
