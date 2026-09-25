/** US2 (T078): laptop shows a resizable nav rail, list and inspector side by side. */
import { expect, test } from '@playwright/test'

import { slotPanes, widthClass } from './layout-helpers'
import { expectNoHorizontalScroll, expectRowCount, inspector, open, table } from './helpers'

test.describe('US2 laptop', () => {
  test('nav, list and inspector are side by side', async ({ page }) => {
    await open(page)
    test.skip((await widthClass(page)) !== 'laptop', 'laptop only')
    await expect(slotPanes(page)).toHaveCount(3)
    await expect(page.getByRole('navigation', { name: 'Sections' })).toBeVisible()
    await expectRowCount(page, 500)

    const list = (await table(page).boundingBox()) ?? { x: 0, y: 0, width: 0, height: 0 }
    const nav = (await page.getByRole('navigation', { name: 'Sections' }).boundingBox()) ?? {
      x: 9999,
      width: 0,
    }
    expect(nav.x + nav.width).toBeLessThanOrEqual(list.x + 1)

    await page.locator('[data-testid="resource-table"] [role="row"][data-row-key]').first().click()
    await expect(inspector(page)).toBeVisible()
    const insp = (await inspector(page).boundingBox()) ?? { x: -1, y: 9999 }
    expect(insp.x).toBeGreaterThanOrEqual(list.x + list.width - 1)
    // Same vertical band: side by side, not stacked.
    expect(Math.abs(insp.y - list.y)).toBeLessThan(80)
    await expectNoHorizontalScroll(page)
  })

  test('the nav is a resizable rail (keyboard operable handle)', async ({ page }) => {
    await open(page)
    test.skip((await widthClass(page)) !== 'laptop', 'laptop only')
    const handle = page.getByRole('separator', { name: 'Resize section 1 and section 2' })
    await handle.focus()
    const before = await handle.getAttribute('aria-valuenow')
    await page.keyboard.press('ArrowLeft')
    await expect.poll(() => handle.getAttribute('aria-valuenow')).not.toBe(before)
  })
})
