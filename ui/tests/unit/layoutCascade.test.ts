import { beforeEach, describe, expect, it } from 'vitest'

import { resolveProfile } from '@/layout/defaults'
import { WIDTH_CLASSES, type WidthClass } from '@/layout/width-class'
import { SECTION } from '@/protocol/commands'
import { assignPanes, initialData, useStore, type PaneId } from '@/store'

const layout = () => useStore.getState().layout
const kindOf = (id: PaneId) => useStore.getState().panes[id]?.kind

/** The slot panes the profile alone would give: what every class below wide must keep showing. */
function profileSlots(cls: WidthClass, ultraPlus = false): PaneId[] {
  const s = useStore.getState()
  return assignPanes(resolveProfile(cls, s.ui.layouts[cls], ultraPlus), s.panes).slotPanes
}

beforeEach(() => {
  useStore.setState({ ...initialData(), send: () => undefined })
})

describe('cascade wiring in the store (044)', () => {
  it('starts the chain at the section list pane', () => {
    expect(layout().cascade).toEqual(layout().slotPanes.slice(0, 1))
    expect(kindOf(layout().cascade[0] ?? '')).toBe('list')
  })

  it('opens a wide row as navigator, section list and empty cascade columns (FR-002)', () => {
    useStore.getState().setWidthClass('wide')
    const { slotPanes, cascade } = layout()
    expect(slotPanes).toHaveLength(4)
    expect(slotPanes.map(kindOf)).toEqual(['nav', 'list', 'inspector', 'inspector'])
    expect(cascade).toEqual([slotPanes[1]])
  })

  it('shows the navigator plus the chain tail once the chain is deep (FR-006)', () => {
    useStore.getState().setWidthClass('ultra')
    const [nav, list, i1, i2, i3] = layout().slotPanes
    // The push path is US2 (T021); Phase 1 only owns the window onto the chain.
    useStore.setState((s) => ({
      layout: { ...s.layout, cascade: [list ?? '', i1 ?? '', i2 ?? ''] },
    }))
    useStore.getState().setWidthClass('ultra', true)
    expect(layout().slotPanes).toEqual([nav, list, i1, i2, i3, expect.any(String)])

    // Narrowing windows onto the tail: the deepest step stays on screen, nothing is remounted.
    useStore.getState().setWidthClass('wide')
    expect(layout().slotPanes).toEqual([nav, list, i1, i2])
    expect(layout().cascade).toEqual([list, i1, i2])
  })

  it('re-reveals the earlier steps when the window grows again (FR-010)', () => {
    useStore.getState().setWidthClass('wide')
    const [nav, list, i1, i2] = layout().slotPanes
    useStore
      .getState()
      .updatePane(i2 ?? '', { search: 'nginx', scrollTop: 240, focusedRow: 'default/web' })
    useStore.setState((s) => ({
      layout: { ...s.layout, cascade: [list ?? '', i1 ?? '', i2 ?? ''] },
    }))
    // At wide the chain fills every slot, so the section list is the step that scrolls off.
    expect(layout().slotPanes).toEqual([nav, list, i1, i2])

    useStore.getState().setWidthClass('laptop')
    useStore.getState().setWidthClass('ultra')
    expect(layout().slotPanes.slice(0, 4)).toEqual([nav, list, i1, i2])
    expect(layout().cascade).toEqual([list, i1, i2])
    // Same panes, same state: a width-class change never remounts a step (FR-010).
    expect(useStore.getState().panes[i2 ?? '']).toMatchObject({
      search: 'nginx',
      scrollTop: 240,
      focusedRow: 'default/web',
    })
  })

  it.each(['phone', 'tablet', 'laptop', 'desktop'] as const)(
    'ignores the chain below wide width, keeping today’s %s slots (FR-009, SC-005)',
    (cls) => {
      useStore.getState().setWidthClass('ultra')
      const deep = layout().slotPanes.slice(1)
      useStore.setState((s) => ({ layout: { ...s.layout, cascade: deep } }))
      useStore.getState().setWidthClass(cls)
      expect(layout().slotPanes).toEqual(profileSlots(cls))
      expect(layout().slotPanes.map(kindOf)).toEqual(resolveProfile(cls).slots.map((s) => s.kind))
    },
  )

  it('gives every visible slot a distinct pane at every class', () => {
    for (const cls of WIDTH_CLASSES) {
      useStore.getState().setWidthClass(cls)
      const ids = layout().slotPanes
      expect(ids).toHaveLength(layout().profile.slots.length)
      expect(new Set(ids).size).toBe(ids.length)
      expect(ids.every((id) => id in useStore.getState().panes)).toBe(true)
    }
  })

  it('resets the chain on a section click (FR-007)', () => {
    useStore.getState().setWidthClass('ultra')
    const [nav, list, i1, i2] = layout().slotPanes
    useStore.setState((s) => ({
      layout: { ...s.layout, cascade: [list ?? '', i1 ?? '', i2 ?? ''] },
    }))
    useStore.getState().navigate(SECTION.pods)
    expect(layout().cascade).toEqual([list])
    expect(layout().slotPanes[0]).toBe(nav)
    expect(layout().slotPanes[1]).toBe(list)
    expect(layout().slotPanes.map(kindOf)).toEqual([
      'nav',
      'list',
      'inspector',
      'inspector',
      'inspector',
    ])
  })

  it('resets the chain on a context change (FR-015)', () => {
    useStore.getState().setWidthClass('ultra')
    const [, list, i1, i2] = layout().slotPanes
    useStore.setState((s) => ({
      layout: { ...s.layout, cascade: [list ?? '', i1 ?? '', i2 ?? ''] },
    }))
    useStore.getState().selectContext('kind-other')
    expect(layout().cascade).toEqual([list])
    expect(layout().slotPanes.map(kindOf)).toEqual([
      'nav',
      'list',
      'inspector',
      'inspector',
      'inspector',
    ])
  })
})
