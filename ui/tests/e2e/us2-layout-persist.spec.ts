/** US2 (T081, T086): layout edits persist per width class via SaveUiConfig and restore on load. */
import { expect, test, type Page } from '@playwright/test'

import { batch, system } from '../fixtures/builders'
import type { UiConfig } from '../../src/protocol/types'
import { widthClass } from './layout-helpers'
import { clearSent, emit, open, sent } from './helpers'

const saves = async (page: Page) =>
  (await sent(page, true)).filter((c) => c.type === 'SaveUiConfig')

test.describe('US2 layout persistence', () => {
  test('resizing a slot at wide sends SaveUiConfig with layouts.wide', async ({ page }) => {
    await page.setViewportSize({ width: 1920, height: 1080 })
    await open(page)
    test.skip((await widthClass(page)) !== 'wide', 'wide only')
    await clearSent(page)
    const handle = page.getByRole('separator', { name: 'Resize section 2 and section 3' })
    await handle.focus()
    for (let i = 0; i < 4; i++) await page.keyboard.press('ArrowRight')

    await expect.poll(async () => (await saves(page)).length).toBeGreaterThan(0)
    const save = (await saves(page)).at(-1) as { data: { ui: UiConfig } }
    expect(save.data.ui.layouts.wide?.slots).toHaveLength(4)
    expect(save.data.ui.layouts.laptop).toBeUndefined()
  })

  test('a saved wide layout is restored at wide; laptop keeps the default', async ({ page }) => {
    const saved: UiConfig = {
      schema: 1,
      theme: { mode: 'dark', accent: 'cyan', effects: 'on', glitch: false },
      density: {},
      layouts: {
        wide: {
          slots: [
            { kind: 'nav', sizePct: 10 },
            { kind: 'list', sizePct: 50 },
            { kind: 'list', sizePct: 15 },
            { kind: 'inspector', sizePct: 25 },
          ],
          dock: { open: true, heightPct: 30, tabs: ['logs', 'events'] },
        },
      },
    }
    const load = batch([
      system(2, {
        type: 'UiConfigLoaded',
        data: { ui: saved, expanded_groups: ['Workloads'], mcp_stdio_enabled: false },
      }),
    ])
    for (const [w, h, cls] of [
      [1920, 1080, 'wide'],
      [1280, 800, 'laptop'],
    ] as const) {
      await page.setViewportSize({ width: w, height: h })
      await open(page)
      await emit(page, load)
      await expect.poll(() => widthClass(page)).toBe(cls)
      const widths = await page
        .locator('[data-slot="pane"][data-slot-index]')
        .evaluateAll((els) => els.map((e) => e.getBoundingClientRect().width))
      expect(widths).toHaveLength(cls === 'wide' ? 4 : 3)
      expect(widths[1]).toBeGreaterThan(widths[2] as number)
      if (cls === 'laptop') expect(widths[0]).toBeLessThan(widths[2] as number)
    }
  })

  /**
   * 044 (FR-011): at wide and above the row is the cascade — one list slot, then inspector
   * columns — so two list panes can only be side by side below wide. Swapping them is unchanged
   * there, which this now covers at laptop with a saved two-list profile.
   */
  test('Pin to slot swaps the two list panes below wide', async ({ page }) => {
    await page.setViewportSize({ width: 1280, height: 800 })
    await open(page)
    test.skip((await widthClass(page)) !== 'laptop', 'laptop only')
    await emit(
      page,
      batch([
        system(2, {
          type: 'UiConfigLoaded',
          data: {
            ui: {
              schema: 1,
              theme: { mode: 'dark', accent: 'cyan', effects: 'on', glitch: false },
              density: {},
              layouts: {
                laptop: {
                  slots: [
                    { kind: 'nav', sizePct: 16 },
                    { kind: 'list', sizePct: 44 },
                    { kind: 'list', sizePct: 40 },
                  ],
                  dock: { open: true, heightPct: 30, tabs: ['logs', 'events'] },
                },
              },
            } satisfies UiConfig,
            expanded_groups: ['Workloads'],
            mcp_stdio_enabled: false,
          },
        }),
      ]),
    )
    const sectionAt = (i: number) =>
      page
        .locator(`[data-slot="pane"][data-slot-index="${String(i)}"]`)
        .getAttribute('data-section')
    await expect.poll(() => page.locator('[data-slot="pane"][data-slot-index]').count()).toBe(3)
    const a = await sectionAt(1)
    const b = await sectionAt(2)
    expect(a).not.toBe(b)
    await page
      .locator('[data-slot="pane"][data-slot-index="1"]')
      .getByTestId('pane-slot-menu')
      .click()
    await page.getByRole('menuitem', { name: 'Pin to slot 3' }).click()
    await expect.poll(() => sectionAt(1)).toBe(b)
    await expect.poll(() => sectionAt(2)).toBe(a)
  })
})
