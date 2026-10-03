/**
 * 048-docker-management T078 (SC-012): 500 containers in one project, and the operator's own
 * input still answered inside 100 ms.
 *
 * The input this scene takes is selection: choosing a group, then a row inside it. Both are the
 * kind of click that grows the DOM — a group expands into its containers, and a row mounts the
 * verbs, the usage panel and the stream controls under it — so they are the two places a list of
 * 500 could be felt. The budget is the same wall-clock, desktop-class one SC-004 uses, which is
 * why this file is named `perf-` and runs only in the serial `*-perf` projects: under the
 * functional suite's parallel workers the same code measures several times this from CPU
 * contention alone.
 */
import { expect, test, type Page } from '@playwright/test'

import { navigateTo, open, sentTypes } from './helpers'

const INPUT_BUDGET_MS = 100

/**
 * The Containers group holds a section of its own name, so the click is scoped to the group's
 * own section list — which is what `navigateTo` does, and it also waits for the phone drawer to
 * finish closing before the next navigation touches it.
 */
async function openContainers(page: Page, label = 'Containers'): Promise<void> {
  await navigateTo(page, label, 'Containers')
}

/**
 * Click-to-render: the press, then the frame after React has flushed whatever the press grew.
 * Two `requestAnimationFrame`s, the same measurement SC-004 takes of a keystroke.
 */
async function clickLatency(page: Page, testId: string): Promise<number> {
  return await page.evaluate(
    (id) =>
      new Promise<number>((resolve) => {
        const el = document.querySelector(`[data-testid="${id}"]`)
        if (!(el instanceof HTMLElement)) {
          resolve(Number.NaN)
          return
        }
        const t0 = performance.now()
        el.click()
        requestAnimationFrame(() => {
          requestAnimationFrame(() => {
            resolve(performance.now() - t0)
          })
        })
      }),
    testId,
  )
}

test.describe('SC-012 containers at 500', () => {
  test('selecting a group and a row inside it each answer under 100 ms', async ({ page }) => {
    await open(page, 'containers-500')
    await openContainers(page)
    await expect(page.getByTestId('container-group-list')).toBeVisible()

    // The list is virtualized, so what is on screen is a window over the 500 rather than 500
    // elements. The count the operator is told is the project's own, and it has to be the truth.
    await expect(page.getByTestId('containers-group-counts-bulk')).toContainText('500')

    const expand = await clickLatency(page, 'containers-group-bulk')
    expect(expand).toBeLessThan(INPUT_BUDGET_MS)

    // A row of the 500, chosen by what the virtualizer actually rendered rather than by name:
    // asking for row 300 would be asking the test to scroll, which is not this budget.
    const firstRow = await page
      .locator('[data-testid^="container-row-"]')
      .first()
      .getAttribute('data-testid')
    expect(firstRow).not.toBeNull()
    const select = await clickLatency(page, firstRow ?? '')
    expect(select).toBeLessThan(INPUT_BUDGET_MS)

    // And the panes the selection mounted are really there — a fast click that rendered nothing
    // would pass the budget and fail the operator.
    await expect(page.locator('[data-testid^="container-streams-"]')).toBeVisible()
  })

  /**
   * T079's half that a stopwatch cannot show: FR-045 and SC-011 say a scene nobody is looking at
   * costs nothing. The only two things in this feature that outlive a single command are the stats
   * sampler and a log follow, and both are owned by a mounted pane — so leaving the section has to
   * end every one of them and start nothing in their place. What is asserted is the wire: after the
   * scene is gone, no further container command is sent at all. The Rust side of the same claim is
   * `stats::tests::stopping_the_sampler_leaves_nothing_ticking` and
   * `stream::tests::killing_a_follow_ends_it`; between them nothing in `container_runtime` runs on a
   * timer, so a wire that goes quiet is a `docker` that is never spawned.
   */
  test('leaving the scene stops every container command, and starts none', async ({ page }) => {
    await open(page, 'containers-500')
    await openContainers(page)
    await expect(page.getByTestId('container-group-list')).toBeVisible()

    // Open the two things that do keep running, so the quiet afterwards is earned rather than
    // trivially true: a selection samples usage, and the logs pane follows.
    await page.getByTestId('containers-group-bulk').click()
    const row = page.locator('[data-testid^="container-row-"]').first()
    await row.click()
    const streams = page.locator('[data-testid^="container-logs-open-"]').first()
    await streams.click()
    await expect(page.locator('[data-testid^="container-logs-scroll-"]')).toBeVisible()

    const nav = page.getByRole('navigation', { name: 'Sections' })
    if (!(await nav.isVisible())) {
      await page.getByRole('button', { name: 'Open sections' }).click()
    }
    await nav.locator('li button', { hasText: 'Pods' }).first().click()
    await expect(page.getByTestId('containers-scene')).toHaveCount(0)

    // The stops are allowed to arrive after the scene is gone — they are how it ends. What must
    // not arrive is a new read, a new stats tick or a new follow.
    const before = await sentTypes(page)
    await page.waitForTimeout(3000)
    const after = await sentTypes(page)
    const started = after
      .slice(before.length)
      .filter((t) => t.startsWith('RefreshContainer') || t.startsWith('StartContainer'))
    expect(started).toEqual([])
  })
})
