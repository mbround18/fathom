/** US2 (T077): phone list -> inspector full screen -> Back keeps scroll; targets and no h-scroll. */
import { expect, test } from '@playwright/test'

import { listScroll, visibleRowKey, widthClass } from './layout-helpers'
import { expectNoHorizontalScroll, expectRowCount, inspector, open, row, table } from './helpers'

const ROWS = '[data-testid="resource-table"] [role="row"][data-row-key]'

test.describe('US2 phone', () => {
  test('inspector is full screen and Back restores the list scroll position', async ({ page }) => {
    await open(page)
    test.skip((await widthClass(page)) !== 'phone', 'phone only')
    await expectRowCount(page, 500)
    await expectNoHorizontalScroll(page)

    await expect.poll(() => listScroll(page, 600)).toBeGreaterThan(300)
    const before = await listScroll(page)
    // Let the throttled scroll write reach the store before the list unmounts.
    await page.waitForTimeout(200)

    await row(page, await visibleRowKey(page)).click()
    const insp = inspector(page)
    await expect(insp).toBeVisible()
    await expect(table(page)).toHaveCount(0)
    const box = await insp.boundingBox()
    expect(box?.width ?? 0).toBeGreaterThanOrEqual((page.viewportSize()?.width ?? 0) - 1)
    await expectNoHorizontalScroll(page)

    await page.getByRole('button', { name: 'Back', exact: true }).click()
    await expect(table(page)).toBeVisible()
    await expect.poll(() => listScroll(page)).toBeGreaterThanOrEqual(before - 5)
  })

  test('touch targets are at least 44px', async ({ page }) => {
    await open(page)
    test.skip((await widthClass(page)) !== 'phone', 'phone only')
    const tabs = page.getByRole('navigation', { name: 'Views' }).getByRole('button')
    const n = await tabs.count()
    expect(n).toBeGreaterThan(0)
    for (let i = 0; i < n; i++) {
      const b = await tabs.nth(i).boundingBox()
      expect(b?.height ?? 0).toBeGreaterThanOrEqual(44)
      expect(b?.width ?? 0).toBeGreaterThanOrEqual(44)
    }
    const rowBox = await page.locator(ROWS).first().boundingBox()
    expect(rowBox?.height ?? 0).toBeGreaterThanOrEqual(44)
  })
})
