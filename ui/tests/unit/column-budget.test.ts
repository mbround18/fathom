/**
 * The per-pane column budget (contracts/layout.md § Column budgets). T076 extends this; the
 * core rules are pinned here because ResourceTable depends on them from US1 on.
 */
import { describe, expect, it } from 'vitest'

import { computeVisibleColumns } from '@/layout/column-budget'

const cols = [
  { id: 'name', priority: 1, minWidth: 180 },
  { id: 'status', priority: 1, minWidth: 110 },
  { id: 'ready', priority: 2, minWidth: 64 },
  { id: 'restarts', priority: 2, minWidth: 80 },
  { id: 'namespace', priority: 2, minWidth: 120 },
  { id: 'node', priority: 4, minWidth: 150 },
] as const

describe('computeVisibleColumns', () => {
  it('always shows priority 1, even when it does not fit', () => {
    expect(computeVisibleColumns(cols, 10)).toEqual(['name', 'status'])
  })

  it('adds columns in ascending priority and stops at the first that does not fit', () => {
    // p1 = 290; ready → 354; restarts → 434; namespace → 554; node → 704.
    expect(computeVisibleColumns(cols, 353)).toEqual(['name', 'status'])
    expect(computeVisibleColumns(cols, 553)).toEqual(['name', 'status', 'ready', 'restarts'])
    // node (p4) would fit at 703 - 120 but never shows while namespace (p2) is hidden.
    expect(computeVisibleColumns(cols, 554)).toEqual([
      'name',
      'status',
      'ready',
      'restarts',
      'namespace',
    ])
  })

  it('never shows a lower priority while a higher one is hidden', () => {
    for (let w = 0; w < 1200; w += 7) {
      const shown = new Set(computeVisibleColumns(cols, w))
      const maxShown = Math.max(...cols.filter((c) => shown.has(c.id)).map((c) => c.priority))
      for (const c of cols) if (c.priority < maxShown) expect(shown.has(c.id)).toBe(true)
    }
  })

  it('keeps declaration order', () => {
    const all = computeVisibleColumns(cols, 5000)
    expect(all).toEqual(cols.map((c) => c.id))
  })

  it('never exceeds the width once past priority 1', () => {
    for (let w = 290; w < 1200; w += 13) {
      const shown = new Set(computeVisibleColumns(cols, w))
      const sum = cols.filter((c) => shown.has(c.id)).reduce((n, c) => n + c.minWidth, 0)
      expect(sum).toBeLessThanOrEqual(w)
    }
  })
})
