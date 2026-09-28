/**
 * 044 US1: a wide or larger window opens on one thing. Contract items 1–3 of
 * `specs/044-cascading-layout/contracts/ui-cascade.md`: after one section click exactly slots 0–2
 * are occupied, the auto-opened step is the section's most relevant row, and a section with
 * nothing to open leaves the next column waiting.
 *
 * Wide and above only: below wide there is no cascade and no auto-open (FR-009), which
 * `cascade-small-windows.spec.ts` (US3) guards.
 */
import { expect, test, type Page, type TestInfo } from '@playwright/test'

import { CASCADE_PODS } from '../fixtures'
import { clearSent, navigateTo, open, PODS, row, sent, table } from './helpers'
import { slotPanes } from './layout-helpers'

const cascadeClass = (info: TestInfo): boolean =>
  /-(wide|ultra|ultra-plus)$/.test(info.project.name)

/** The pane in slot `i` (`[data-slot-index]` is only set for the visible row). */
const slot = (page: Page, i: number) =>
  page.locator(`[data-slot="pane"][data-slot-index="${String(i)}"]`)

/** The step the auto-open filled (contract "Slots and test ids"). */
const autoOpened = (page: Page) => page.locator('[data-slot="pane"][data-auto-open="true"]')

// Playwright requires the fixtures argument to be destructured, even when none is used.
// eslint-disable-next-line no-empty-pattern
test.beforeEach(({}, info) => {
  test.skip(!cascadeClass(info), 'the cascade starts at wide width (FR-009)')
})

test.describe('044 US1 clean start on a wide screen', () => {
  test('one section click occupies slots 0-2 and leaves the rest waiting (contract 1)', async ({
    page,
  }) => {
    await open(page, 'cascade-unhealthy')
    await navigateTo(page, 'Pods')
    await expect(table(page, PODS)).toBeVisible()

    // Slot 0 the navigator, slot 1 the section (FR-001), slot 2 the auto-opened item (FR-003).
    await expect(slot(page, 0)).toHaveAttribute('data-pane-kind', 'nav')
    await expect(slot(page, 1)).toHaveAttribute('data-pane-kind', 'list')
    await expect(slot(page, 1)).toHaveAttribute('data-section', PODS)
    await expect(slot(page, 2)).toHaveAttribute('data-pane-kind', 'inspector')
    await expect(slot(page, 2)).toHaveAccessibleName('cascade-err')

    // Every remaining column is waiting for the next click (FR-002, FR-012).
    const count = await slotPanes(page).count()
    expect(count).toBeGreaterThanOrEqual(4)
    for (let i = 3; i < count; i++) {
      await expect(slot(page, i).getByTestId('cascade-empty')).toBeVisible()
    }
    await expect(page.getByTestId('cascade-empty')).toHaveCount(count - 3)

    // Nothing the operator did not ask for: no prefilled tool pane and no second list (FR-002).
    expect(
      await page.locator('[data-slot="pane"][data-slot-index][data-pane-kind="tool"]').count(),
    ).toBe(0)
    expect(
      await page.locator('[data-slot="pane"][data-slot-index][data-pane-kind="list"]').count(),
    ).toBe(1)
  })

  const relevance = [
    { scenario: 'cascade-unhealthy', expected: 'cascade-err', why: 'the first err row' },
    { scenario: 'cascade-warning', expected: 'cascade-warn', why: 'the first warn row' },
    { scenario: 'cascade-healthy', expected: 'cascade-ok-first', why: 'the first row' },
  ] as const

  for (const { scenario, expected, why } of relevance) {
    test(`the auto-opened step is ${why} (contract 2, ${scenario})`, async ({ page }) => {
      await open(page, scenario)
      await navigateTo(page, 'Pods')
      await expect(table(page, PODS)).toBeVisible()

      await expect(autoOpened(page)).toHaveCount(1)
      await expect(autoOpened(page)).toHaveAttribute('data-slot-index', '2')
      await expect(autoOpened(page)).toHaveAccessibleName(expected)
      // The list shows the same row as the open one, exactly as a click would leave it.
      await expect(row(page, `default/${expected}`)).toHaveAttribute('aria-selected', 'true')
    })
  }

  test('the marker drops as soon as the operator clicks in the cascade (contract "Slots")', async ({
    page,
  }) => {
    await open(page, 'cascade-unhealthy')
    await navigateTo(page, 'Pods')
    await expect(autoOpened(page)).toHaveCount(1)
    await row(page, 'default/cascade-ok-last').click()
    await expect(autoOpened(page)).toHaveCount(0)
    await expect(slot(page, 2)).toHaveAccessibleName('cascade-ok-last')
  })

  test('an empty section leaves the next column waiting (contract 3)', async ({ page }) => {
    await open(page, 'cascade-empty')
    await navigateTo(page, 'Pods')
    await expect(table(page, PODS)).toHaveAttribute('data-row-count', '0')
    await expect(slot(page, 2).getByTestId('cascade-empty')).toBeVisible()
    await expect(autoOpened(page)).toHaveCount(0)
    // T052a: the first cascade column waits for a row click, and its header says so.
    await expect(slot(page, 2)).toHaveAccessibleName('Selected item')
  })

  test('a listless destination leaves the next column waiting (contract 3)', async ({ page }) => {
    await open(page, 'cascade-unhealthy')
    await navigateTo(page, 'App settings', 'Settings')
    await expect(slot(page, 2).getByTestId('cascade-empty')).toBeVisible()
    await expect(autoOpened(page)).toHaveCount(0)

    await navigateTo(page, 'Topology', 'Cluster')
    await expect(slot(page, 2).getByTestId('cascade-empty')).toBeVisible()
    await expect(autoOpened(page)).toHaveCount(0)
  })

  test('the auto-open emits only the read commands a row click emits (contract "Commands")', async ({
    page,
  }) => {
    await open(page, 'cascade-unhealthy')
    await clearSent(page)
    await navigateTo(page, 'Pods')
    await expect(autoOpened(page)).toHaveCount(1)

    // Everything the auto-open sent beyond the section's own list refresh.
    const auto = (await sent(page)).filter(
      (c) => c.type !== 'RefreshPods' && c.type !== 'SetActiveSection',
    )
    expect(auto.some((c) => c.type === 'SelectPod')).toBe(true)

    // The same row, clicked by hand, sends exactly the same reads and nothing else.
    await clearSent(page)
    await row(page, `default/${CASCADE_PODS.unhealthy[2].name}`).click()
    await expect
      .poll(async () => (await sent(page)).filter((c) => c.type === 'SelectPod').length)
      .toBeGreaterThan(0)
    const manual = await sent(page)
    expect(auto.map((c) => c.type).sort()).toEqual(manual.map((c) => c.type).sort())
    expect(auto).toEqual(manual)
  })
})
