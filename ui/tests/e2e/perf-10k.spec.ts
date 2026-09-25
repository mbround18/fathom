/**
 * T119 (SC-004): with 10,000 pods loaded and list updates streaming every 250 ms, a search
 * keystroke renders in under 100 ms and no long task exceeds 100 ms. The budget is a
 * desktop-class one, so only the `wide` project of each engine runs it (`chromium-wide`,
 * `webkit-wide`).
 */
import { expect, test } from '@playwright/test'

import { pods10k } from '../fixtures'
import { expectRowCount, open, searchBox } from './helpers'

const KEYSTROKE_BUDGET_MS = 100
const LONG_TASK_BUDGET_MS = 100

test.describe('SC-004 perf 10k pods', () => {
  test.beforeEach((_fixtures, info) => {
    test.skip(!info.project.name.endsWith('-wide'), 'perf budget is measured on the wide project')
  })

  test('search keystroke-to-render < 100 ms and no long task > 100 ms under 250 ms updates', async ({
    page,
  }) => {
    await open(page, 'pods-10k')
    await expectRowCount(page, 10_000)

    // Long-task observer, installed after the initial 10k load so the measured window is the
    // steady state (streaming updates + typing), not the one-off first paint.
    await page.evaluate(() => {
      const w = window as unknown as { __longTasks: number[] }
      w.__longTasks = []
      new PerformanceObserver((list) => {
        for (const e of list.getEntries()) w.__longTasks.push(e.duration)
      }).observe({ entryTypes: ['longtask'] })
    })

    // Update stream: every 250 ms re-emit the 10k list with a moving slice of restart counts.
    const batch = pods10k()
    await page.evaluate((b) => {
      let tick = 0
      const w = window as unknown as { __updates: number }
      w.__updates = 0
      setInterval(() => {
        tick += 1
        const copy = structuredClone(b)
        for (const env of copy.envelopes) {
          env.seq += tick * 1000
          const ev = env.event as { data?: { data?: { restarts: number }[] } }
          const rows = ev.data?.data
          if (Array.isArray(rows)) {
            for (let i = tick % 50; i < rows.length; i += 50) {
              const r = rows[i]
              if (r) r.restarts += tick
            }
          }
        }
        window.__fathomFake?.emit(copy)
        w.__updates += 1
      }, 250)
    }, batch)

    await searchBox(page).click()
    const latencies: number[] = []
    for (const ch of 'web-a') {
      // Keystroke-to-render: key press to the frame after React has flushed the filtered list.
      const ms = await page.evaluate(
        (c) =>
          new Promise<number>((resolve) => {
            const el = document.activeElement as HTMLInputElement
            const t0 = performance.now()
            // Native setter bypasses React's value tracker so the synthetic input event is seen as a change.
            // eslint-disable-next-line @typescript-eslint/unbound-method
            const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value')?.set
            setter?.call(el, el.value + c)
            el.dispatchEvent(
              new InputEvent('input', { bubbles: true, data: c, inputType: 'insertText' }),
            )
            requestAnimationFrame(() =>
              requestAnimationFrame(() => resolve(performance.now() - t0)),
            )
          }),
        ch,
      )
      latencies.push(ms)
      await page.waitForTimeout(300) // straddle update ticks
    }
    await page.waitForTimeout(1000)

    const { longTasks, updates } = await page.evaluate(() => {
      const w = window as unknown as { __longTasks: number[]; __updates: number }
      return { longTasks: w.__longTasks, updates: w.__updates }
    })
    expect(updates).toBeGreaterThanOrEqual(4)
    expect(Math.max(...latencies), `keystroke latencies ${latencies.join(', ')}`).toBeLessThan(
      KEYSTROKE_BUDGET_MS,
    )
    expect(Math.max(0, ...longTasks), `long tasks ${longTasks.join(', ')}`).toBeLessThan(
      LONG_TASK_BUDGET_MS,
    )
  })
})
