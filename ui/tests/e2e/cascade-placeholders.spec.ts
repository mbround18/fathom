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

    // T052a: a waiting column's own header names the job, not the widget — never "Inspector".
    for (let i = 3; i < count; i++) {
      const header = slot(page, i).getByRole('heading', { level: 2 })
      await expect(header).toHaveText('Next level')
      await expect(slot(page, i)).toHaveAttribute('aria-label', 'Next level')
    }
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
    await expect(table(page, PODS).locator('[role="row"][aria-current="true"]')).toHaveCount(0)
    await expect(row(page, 'default/cascade-warn')).not.toHaveAttribute('aria-current', 'true')
  })

  test('the keyboard reaches every column and its dividers (T052)', async ({ page }) => {
    await open(page, 'cascade-unhealthy')
    await navigateTo(page, 'Pods')
    await expect(slot(page, 2)).toHaveAccessibleName('cascade-err')

    // Every divider between visible slots keeps its accessible name, cascade columns included, so
    // a keyboard-only operator can still find and resize them.
    const count = await slotPanes(page).count()
    for (let i = 1; i < count; i++) {
      await expect(
        page.getByRole('separator', {
          name: `Resize section ${String(i)} and section ${String(i + 1)}`,
        }),
      ).toHaveCount(1)
    }

    // Tabbing forward from the navigator reaches the occupied cascade column: no column is a
    // keyboard dead end and none of the waiting ones swallow the focus order.
    await slot(page, 0).getByRole('button').first().focus()
    const reached = await page.evaluate(() => {
      const slotOf = (el: Element | null): string | null =>
        el?.closest('[data-slot="pane"]')?.getAttribute('data-slot-index') ?? null
      const seen = new Set<string>()
      for (let i = 0; i < 400; i++) {
        const at = slotOf(document.activeElement)
        if (at !== null) seen.add(at)
        if (seen.has('2')) break
        // Playwright's keyboard is faster than this loop needs; step the sequence natively.
        const focusables = [
          ...document.querySelectorAll<HTMLElement>(
            'a[href], button:not([disabled]), input:not([disabled]), [tabindex]:not([tabindex="-1"])',
          ),
        ].filter((e) => e.offsetParent !== null)
        const next = focusables[focusables.indexOf(document.activeElement as HTMLElement) + 1]
        if (!next) break
        next.focus()
      }
      return [...seen]
    })
    expect(reached).toContain('2')
  })
})
