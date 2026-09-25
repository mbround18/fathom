/** Shared helpers for the US2 layout specs. */
import type { Page } from '@playwright/test'

import { PODS } from './helpers'

/** Set (or, with no argument, read) the virtualised list body's scrollTop for `section`. */
export async function listScroll(
  page: Page,
  top?: number,
  section: string = PODS,
): Promise<number> {
  return page.evaluate(
    ([sec, t]) => {
      const root = document.querySelector(
        `[data-testid="resource-table"][data-section="${sec}"]`,
      )
      const el = [...(root?.querySelectorAll<HTMLElement>('*') ?? [])].find((e) => {
        const oy = getComputedStyle(e).overflowY
        return (oy === 'auto' || oy === 'scroll') && e.scrollHeight > e.clientHeight
      })
      if (!el) return -1
      if (typeof t === 'number') el.scrollTop = t
      return el.scrollTop
    },
    [section, top] as const,
  )
}

/**
 * A row actually inside the list viewport (the virtualiser also renders overscan rows outside it;
 * clicking one of those would scroll the list). Returns its `data-row-key`.
 */
export async function visibleRowKey(page: Page, section: string = PODS): Promise<string> {
  return page.evaluate((sec) => {
    const root = document.querySelector(
      `[data-testid="resource-table"][data-section="${sec}"]`,
    )
    const rows = [...(root?.querySelectorAll<HTMLElement>('[role="row"][data-row-key]') ?? [])]
    const scroller = [...(root?.querySelectorAll<HTMLElement>('*') ?? [])].find((e) => {
      const oy = getComputedStyle(e).overflowY
      return (oy === 'auto' || oy === 'scroll') && e.scrollHeight > e.clientHeight
    })
    const box = (scroller ?? (root as HTMLElement)).getBoundingClientRect()
    const r = rows.find((e) => {
      const b = e.getBoundingClientRect()
      return b.top >= box.top + 4 && b.bottom <= box.bottom - 4
    })
    return r?.dataset.rowKey ?? ''
  }, section)
}

/** Visible slot panes (`[data-slot="pane"][data-slot-index]`). */
export const slotPanes = (page: Page) => page.locator('[data-slot="pane"][data-slot-index]')

export const widthClass = (page: Page) => page.locator('html').getAttribute('data-width-class')
