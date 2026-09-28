/**
 * US2 (T079): ultra shows five or more slots plus the dock, and more than wide does.
 *
 * 044 (FR-011): the ultra row is the cascade — navigator, the selected section's list, then
 * inspector columns. The second list pane and the docked tool slot this test once expected are
 * gone; the tools are navigator sections (043) and the extra columns are drill-in levels.
 */
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
    expect(kinds.slice(0, 3)).toEqual(['nav', 'list', 'inspector'])
    expect(kinds.filter((k) => k === 'list')).toHaveLength(1)
    expect(kinds.filter((k) => k === 'inspector').length).toBeGreaterThanOrEqual(3)
    expect(kinds).not.toContain('tool')
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
