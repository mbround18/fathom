/**
 * 044 US4: the waiting columns invite the next click. FR-012 (every empty cascade column explains
 * itself), FR-013 (filling or emptying a column keeps the widths the operator dragged) and FR-014
 * (a step whose object is gone says so instead of jumping to another item).
 *
 * Wide and above only: below wide there is no cascade (FR-009), which
 * `cascade-small-windows.spec.ts` guards.
 */
import { expect, test, type Page, type TestInfo } from '@playwright/test'

import { CASCADE_PODS, pods } from '../fixtures'
import {
  emit,
  expectRowCount,
  navigateTo,
  open,
  PODS,
  row,
  settleFakeReplies,
  table,
} from './helpers'
import { slotPanes } from './layout-helpers'

const EMPTY_COPY = 'Follow a link to open the next level here'

const cascadeClass = (info: TestInfo): boolean =>
  /-(wide|ultra|ultra-plus)$/.test(info.project.name)

const slot = (page: Page, i: number) =>
  page.locator(`[data-slot="pane"][data-slot-index="${String(i)}"]`)

/** Every visible slot's rendered width, left to right. */
const slotWidths = (page: Page): Promise<number[]> =>
  slotPanes(page).evaluateAll((els) => els.map((e) => e.getBoundingClientRect().width))

// Playwright requires the fixtures argument to be destructured, even when none is used.
// eslint-disable-next-line no-empty-pattern
test.beforeEach(({}, info) => {
  test.skip(!cascadeClass(info), 'the cascade starts at wide width (FR-009)')
})

test.describe('044 US4 cascade placeholders', () => {
  test('every empty column says what the next click will open (FR-012)', async ({ page }) => {
    await open(page, 'cascade-unhealthy')
    await navigateTo(page, 'Pods')
    await expect(table(page, PODS)).toBeVisible()

    // Slots 0-2 are occupied (US1), so every slot from 3 on is a waiting column.
    const count = await slotPanes(page).count()
    expect(count).toBeGreaterThanOrEqual(4)
    const empties = page.getByTestId('cascade-empty')
    await expect(empties).toHaveCount(count - 3)
    for (let i = 3; i < count; i++) {
      const placeholder = slot(page, i).getByTestId('cascade-empty')
      await expect(placeholder).toBeVisible()
      await expect(placeholder).toContainText(EMPTY_COPY)
    }
    // Announced rather than silent decoration, and waiting rather than broken: a polite status,
    // never an alert (FR-012, T052).
    await expect(empties.first().getByRole('status')).toBeVisible()
    await expect(empties.first().getByRole('alert')).toHaveCount(0)
  })

  test('a filled column keeps the widths the operator dragged (FR-013)', async ({ page }) => {
    // An empty section leaves slot 2 waiting too (FR-004), so the fill below is a real
    // empty → occupied transition without needing a drill-in.
    await open(page, 'cascade-empty')
    await navigateTo(page, 'Pods')
    await expect(table(page, PODS)).toHaveAttribute('data-row-count', '0')
    await expect(slot(page, 2).getByTestId('cascade-empty')).toBeVisible()

    const before = await slotWidths(page)
    const handle = page.getByRole('separator', { name: 'Resize section 1 and section 2' })
    await handle.focus()
    for (let i = 0; i < 6; i++) await page.keyboard.press('ArrowRight')
    await expect.poll(async () => (await slotWidths(page))[0]).toBeGreaterThan(before[0] ?? 0)
    const dragged = await slotWidths(page)

    // Deployments has rows, so its auto-open fills the column that was waiting (FR-003).
    await navigateTo(page, 'Deployments')
    await expect(slot(page, 2)).toHaveAttribute('data-pane-kind', 'inspector')
    await expect(slot(page, 2).getByTestId('cascade-empty')).toHaveCount(0)

    // The operator's sizes survive the fill, to the pixel.
    expect(await slotWidths(page)).toEqual(dragged)
  })

  test('a step whose object is gone says so and does not jump (FR-014)', async ({ page }) => {
    await open(page, 'cascade-unhealthy')
    await navigateTo(page, 'Pods')
    await expect(slot(page, 2)).toHaveAccessibleName('cascade-err')

    // The pod the cascade opened is deleted from the cluster: the list refreshes without it. The
    // scenario's own queued replies are drained first, so none of them can land on top of this one.
    await settleFakeReplies(page)
    await emit(
      page,
      pods(
        'kind-a',
        CASCADE_PODS.unhealthy.filter((p) => p.name !== 'cascade-err'),
        99,
      ),
    )
    await expectRowCount(page, 3)

    const gone = slot(page, 2).getByTestId('cascade-gone')
    await expect(gone).toBeVisible()
    await expect(gone).toContainText('cascade-err')
    await expect(gone).toContainText('default')
    // The step keeps its target, and nothing else is selected in its place (FR-014).
    await expect(slot(page, 2)).toHaveAccessibleName('cascade-err')
    await expect(table(page, PODS).locator('[role="row"][aria-selected="true"]')).toHaveCount(0)
    await expect(row(page, 'default/cascade-warn')).toHaveAttribute('aria-selected', 'false')
  })
})
