/**
 * 044 US2: drilling in cascades to the right. Contract items 4–9 of
 * `specs/044-cascading-layout/contracts/ui-cascade.md`: a drill-in fills the next empty column and
 * leaves the one it came from, an overflowing chain shifts left without touching the navigator or
 * the section list, a click in an earlier column empties the ones after it, a section or context
 * change returns the row to the US1 state, the dock is untouched — and a width-class change keeps
 * every step, its selection, its search text and its scroll position (FR-010, SC-004).
 *
 * Wide and above only: below wide there is no cascade (FR-009), which `cascade-small-windows.spec.ts`
 * (US3) guards.
 */
import { expect, test, type Locator, type Page, type TestInfo } from '@playwright/test'

import { chooseScope, expectScope, navigateTo, open, PODS, row, searchBox, table } from './helpers'
import { listScroll, slotPanes, visibleRowKey, widthClass } from './layout-helpers'

const cascadeClass = (info: TestInfo): boolean =>
  /-(wide|ultra|ultra-plus)$/.test(info.project.name)

/** The pane in slot `i` (`[data-slot-index]` is only set for the visible row). */
const slot = (page: Page, i: number) =>
  page.locator(`[data-slot="pane"][data-slot-index="${String(i)}"]`)

/** The occupied cascade columns, left to right (contract "Slots and test ids"). */
const steps = (page: Page) => page.getByTestId('cascade-step')

/** The cascade column numbered `n` (n from 1 at slot 2). */
const step = (page: Page, n: number) =>
  page.locator(`[data-testid="cascade-step"][data-step="${String(n)}"]`)

/** The cascade column showing `name`, whichever number it currently carries. */
const stepFor = (page: Page, name: string) =>
  page.locator(`[data-testid="cascade-step"][aria-label="${name}"]`)

/** The names the cascade columns show, left to right. */
async function chain(page: Page): Promise<(string | null)[]> {
  return steps(page).evaluateAll((els) => els.map((e) => e.getAttribute('aria-label')))
}

/** Open the chain scenario at Pods with `chain-1` in the first cascade column. */
async function openChain(page: Page): Promise<void> {
  await open(page, 'cascade-chain')
  await navigateTo(page, 'Pods')
  await expect(table(page, PODS)).toBeVisible()
  await row(page, 'default/chain-1').click()
  await expect(step(page, 1)).toHaveAccessibleName('chain-1')
  await expect(steps(page)).toHaveCount(1)
}

/** Follow `to`'s related-resource link from the column showing `from`. */
async function drill(page: Page, from: string, to: string): Promise<void> {
  await stepFor(page, from).getByRole('button').filter({ hasText: to }).first().click()
  await expect(stepFor(page, to)).toBeVisible()
}

/** Walk `chain-1` → `chain-2` → … until `depth` columns are occupied. */
async function drillTo(page: Page, depth: number): Promise<void> {
  for (let n = 1; n < depth; n++) await drill(page, `chain-${String(n)}`, `chain-${String(n + 1)}`)
  await expect(steps(page)).toHaveCount(depth)
}

/** How many cascade columns this width class offers (slots 2 and up). */
async function columns(page: Page): Promise<number> {
  return (await slotPanes(page).count()) - 2
}

const dock = (page: Page): Locator => page.getByTestId('dock-tabs')

/** The dock's tab labels, its open state and its height (FR-016). */
async function dockState(page: Page): Promise<{ tabs: (string | null)[]; height: number }> {
  const tabs = await dock(page)
    .getByRole('tab')
    .evaluateAll((els) => els.map((e) => e.textContent))
  const box = await dock(page).boundingBox()
  return { tabs, height: Math.round(box?.height ?? 0) }
}

// Playwright requires the fixtures argument to be destructured, even when none is used.
// eslint-disable-next-line no-empty-pattern
test.beforeEach(({}, info) => {
  test.skip(!cascadeClass(info), 'the cascade starts at wide width (FR-009)')
})

test.describe('044 US2 drilling in cascades to the right', () => {
  test('each drill-in fills the next empty column and leaves the one before it (contract 4)', async ({
    page,
  }) => {
    await openChain(page)
    const free = await columns(page)

    await drill(page, 'chain-1', 'chain-2')
    // The column it came from still shows its own object, the new one is numbered one higher.
    await expect(step(page, 1)).toHaveAccessibleName('chain-1')
    await expect(step(page, 2)).toHaveAccessibleName('chain-2')
    await expect(slot(page, 3)).toHaveAttribute('data-step', '2')

    // Every column still free is waiting, and no column was skipped.
    await expect(page.getByTestId('cascade-empty')).toHaveCount(free - 2)
    for (let n = 3; n <= free; n++) {
      await drill(page, `chain-${String(n - 1)}`, `chain-${String(n)}`)
      await expect(step(page, n)).toHaveAccessibleName(`chain-${String(n)}`)
    }
    await expect(page.getByTestId('cascade-empty')).toHaveCount(0)
    await expect(steps(page)).toHaveCount(free)
  })

  test('one drill-in too many shifts the chain left and keeps slots 0 and 1 (contract 5)', async ({
    page,
  }) => {
    await openChain(page)
    const free = await columns(page)
    await drillTo(page, free)
    const before = await chain(page)

    const deepest = `chain-${String(free)}`
    const next = `chain-${String(free + 1)}`
    await drill(page, deepest, next)

    // The navigator and the section list never move (FR-001, FR-006).
    await expect(slot(page, 0)).toHaveAttribute('data-pane-kind', 'nav')
    await expect(slot(page, 1)).toHaveAttribute('data-section', PODS)
    // The oldest drill-down left the screen, the rest shifted one place left, the new one is last.
    expect(await chain(page)).toEqual([...before.slice(1), next])
    await expect(step(page, free)).toHaveAccessibleName(next)
    await expect(steps(page)).toHaveCount(free)
    await expect(page.getByTestId('cascade-empty')).toHaveCount(0)
  })

  test('another row in the section list replaces column 2 and empties the rest (contract 6)', async ({
    page,
  }) => {
    await openChain(page)
    const free = await columns(page)
    await drillTo(page, Math.min(3, free))

    await row(page, 'default/chain-9').click()
    await expect(step(page, 1)).toHaveAccessibleName('chain-9')
    await expect(steps(page)).toHaveCount(1)
    await expect(page.getByTestId('cascade-empty')).toHaveCount(free - 1)
  })

  test('another section returns the whole row to the US1 state (contract 6)', async ({ page }) => {
    await openChain(page)
    await drillTo(page, Math.min(3, await columns(page)))

    // A section with nothing to open leaves every cascade column waiting (FR-002, FR-004).
    await navigateTo(page, 'App settings', 'Settings')
    await expect(steps(page)).toHaveCount(0)
    await expect(page.getByTestId('cascade-empty')).toHaveCount(await columns(page))

    // And a section with a list opens exactly its most relevant item again (FR-003).
    await navigateTo(page, 'Pods')
    await expect(steps(page)).toHaveCount(1)
    await expect(step(page, 1)).toHaveAccessibleName('chain-1')
    await expect(step(page, 1)).toHaveAttribute('data-auto-open', 'true')
  })

  test('a context change returns the row to the US1 state for the new context (contract 8)', async ({
    page,
  }) => {
    await openChain(page)
    await drillTo(page, Math.min(3, await columns(page)))

    await chooseScope(page, 'Context', 'kind-b')
    await expectScope(page, 'Context', 'kind-b')
    // No column keeps kind-a's object (FR-015).
    await expect(page.locator('[data-testid="cascade-step"][aria-label^="chain-"]')).toHaveCount(0)
    await expect(steps(page)).toHaveCount(0)
    await expect(page.getByTestId('cascade-empty')).toHaveCount(await columns(page))
  })

  test('the dock is unaffected by the whole walk (contract 9, FR-016)', async ({ page }) => {
    await openChain(page)
    const before = await dockState(page)
    expect(before.tabs.length).toBeGreaterThan(0)

    await drillTo(page, await columns(page))
    await drill(
      page,
      `chain-${String(await columns(page))}`,
      `chain-${String((await columns(page)) + 1)}`,
    )
    await row(page, 'default/chain-9').click()

    await expect(dock(page)).toBeVisible()
    expect(await dockState(page)).toEqual(before)
  })
})

/**
 * T024 (contract 7, SC-004): a three-step cascade crossing 2560 px and 1920 px in both directions.
 * Driven from the `ultra` project alone — the viewports are set by hand, so running it once per
 * engine covers every boundary rather than repeating the same walk at each class.
 */
test.describe('044 US2 width boundaries', () => {
  // eslint-disable-next-line no-empty-pattern
  test.beforeEach(({}, info) => {
    test.skip(!/-ultra$/.test(info.project.name), 'drives every boundary from one project')
  })

  test('every step, its selection, search text and scroll position survive a resize', async ({
    page,
  }) => {
    await page.setViewportSize({ width: 2560, height: 1440 })
    await openChain(page)
    await drillTo(page, 3)

    // State the resize must not lose: the section list's filter, selection and scroll position.
    await searchBox(page, PODS).fill('chain-1')
    await expect
      .poll(async () => Number(await table(page, PODS).getAttribute('data-row-count')))
      .toBeGreaterThan(1)
    const rows = (await table(page, PODS).getAttribute('data-row-count')) ?? ''
    await expect.poll(() => listScroll(page, 300)).toBeGreaterThan(100)
    await page.waitForTimeout(200)
    const scrolled = await listScroll(page)
    // The selected row is scrolled out of the virtualiser, so the column showing it stands in for
    // the selection and the row under the viewport's top edge stands in for the scroll offset.
    const topRow = await visibleRowKey(page)
    const deep = await chain(page)
    expect(deep).toEqual(['chain-1', 'chain-2', 'chain-3'])

    const at = async (width: number, cls: string): Promise<void> => {
      await page.setViewportSize({ width, height: 1080 })
      await expect.poll(() => widthClass(page)).toBe(cls)
    }
    const intact = async (visible: string[]): Promise<void> => {
      // Polled: the row re-slots a tick after the width class changes, so the columns settle late.
      await expect.poll(() => chain(page)).toEqual(visible)
      await expect(searchBox(page, PODS)).toHaveValue('chain-1')
      await expect(table(page, PODS)).toHaveAttribute('data-row-count', rows)
      await expect.poll(() => listScroll(page)).toBeGreaterThanOrEqual(scrolled - 5)
      expect(await visibleRowKey(page)).toBe(topRow)
    }

    // Down over 2560: wide offers two cascade columns, so the row shows the deepest two steps.
    await at(1920, 'wide')
    await intact(['chain-2', 'chain-3'])

    // Down over 1920: no cascade below wide (FR-009), and the state is still there.
    await at(1600, 'desktop')
    await expect(steps(page)).toHaveCount(0)
    await expect(page.getByTestId('cascade-empty')).toHaveCount(0)
    await expect(searchBox(page, PODS)).toHaveValue('chain-1')
    await expect.poll(() => listScroll(page)).toBeGreaterThanOrEqual(scrolled - 5)

    // Back up: the hidden steps reappear from the right end, nothing was remounted.
    await at(1920, 'wide')
    await intact(['chain-2', 'chain-3'])
    await at(2560, 'ultra')
    await intact(['chain-1', 'chain-2', 'chain-3'])

    // And the chain still grows from where it left off.
    await drill(page, 'chain-3', 'chain-4')
    expect(await chain(page)).toEqual(['chain-2', 'chain-3', 'chain-4'])
  })
})
