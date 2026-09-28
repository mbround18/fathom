import { describe, expect, it } from 'vitest'

import { CASCADE_FIRST_SLOT, pushStep, resetTo, slotsFor, truncateAt } from '@/layout/cascade'

/** The nav pane and the four cascade columns of a wide row, named for readability. */
const NAV = 'nav-1'
const LIST = 'list-1'

describe('cascade algebra (044)', () => {
  it('starts a chain at the section list pane', () => {
    expect(resetTo(LIST)).toEqual([LIST])
    expect(CASCADE_FIRST_SLOT).toBe(2)
  })

  it('appends a drill-in step (FR-005)', () => {
    let chain = resetTo(LIST)
    chain = pushStep(chain, 'inspector-1')
    chain = pushStep(chain, 'inspector-2')
    expect(chain).toEqual([LIST, 'inspector-1', 'inspector-2'])
  })

  it('re-enters a step already in the chain instead of duplicating it', () => {
    const chain = [LIST, 'inspector-1', 'inspector-2', 'inspector-3']
    expect(pushStep(chain, 'inspector-1')).toEqual([LIST, 'inspector-1'])
    expect(new Set(pushStep(chain, 'inspector-3')).size).toBe(4)
  })

  it('truncates when a row is picked in a column that is not the last (FR-007)', () => {
    const chain = [LIST, 'inspector-1', 'inspector-2', 'inspector-3']
    expect(truncateAt(chain, 1)).toEqual([LIST, 'inspector-1'])
    expect(truncateAt(chain, 0)).toEqual([LIST])
    expect(truncateAt(chain, 3)).toEqual(chain)
    expect(chain).toHaveLength(4)
  })

  it('shows the whole chain and pads the tail with the spare panes (FR-002)', () => {
    expect(slotsFor([LIST], NAV, 4, ['inspector-1', 'inspector-2'])).toEqual([
      NAV,
      LIST,
      'inspector-1',
      'inspector-2',
    ])
    // Without spare panes the tail is simply empty, never a duplicate of an occupied column.
    expect(slotsFor([LIST], NAV, 4)).toEqual([NAV, LIST])
  })

  it('scrolls only the steps when the chain outgrows the slots (FR-006)', () => {
    const chain = [LIST, 'inspector-1', 'inspector-2', 'inspector-3']
    // The navigator and the section list are pinned; the oldest *step* is what leaves.
    expect(slotsFor(chain, NAV, 4)).toEqual([NAV, LIST, 'inspector-2', 'inspector-3'])
    // The deepest step is always the last slot, so a drill-in never lands off screen.
    const deeper = slotsFor([...chain, 'inspector-4'], NAV, 4)
    expect(deeper.at(-1)).toBe('inspector-4')
    expect(deeper.slice(0, 2)).toEqual([NAV, LIST])
  })

  it('re-reveals earlier steps when the window grows (FR-010)', () => {
    const chain = [LIST, 'inspector-1', 'inspector-2', 'inspector-3']
    expect(slotsFor(chain, NAV, 4)).not.toContain('inspector-1')
    expect(slotsFor(chain, NAV, 5)).toEqual([NAV, ...chain])
    expect(slotsFor(chain, NAV, 6, ['inspector-9'])).toEqual([NAV, ...chain, 'inspector-9'])
  })

  it('keeps slot ids unique and never longer than the slot count', () => {
    const chain = [LIST, 'inspector-1', 'inspector-2']
    const slots = slotsFor(chain, NAV, 5, ['inspector-1', 'inspector-2', 'inspector-3'])
    expect(slots).toHaveLength(5)
    expect(new Set(slots).size).toBe(5)
    expect(slotsFor(chain, NAV, 1, ['inspector-3'])).toEqual([NAV])
    expect(slotsFor(chain, NAV, 0)).toEqual([])
  })
})
