import { beforeEach, describe, expect, it } from 'vitest'

import { resolveProfile } from '@/layout/defaults'
import { WIDTH_CLASSES, type WidthClass } from '@/layout/width-class'
import { SECTION } from '@/protocol/commands'
import type { ObjectRef } from '@/resources/types'
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

  it.each(['phone', 'tablet', 'laptop', 'desktop'] as const)(
    'derives %s slot panes from the profile alone, whatever the chain holds (T032, FR-009)',
    (cls) => {
      // The regression guard for the gating: below wide, `slotPanes` is `assignPanes(profile)`
      // verbatim — the same derivation as before 044 — after every action that rewrites the chain
      // at wide and above. If one of them starts windowing the chain here, this fails.
      useStore.getState().setWidthClass(cls)
      const baseline = [...layout().slotPanes]
      expect(baseline).toEqual(profileSlots(cls))

      useStore.getState().navigate(SECTION.deployments)
      expect(layout().slotPanes).toEqual(baseline)
      useStore.getState().inspect('Deployment', 'default', 'web')
      expect(layout().slotPanes).toEqual(baseline)
      useStore.getState().selectContext('kind-other')
      expect(layout().slotPanes).toEqual(baseline)
      expect(layout().slotPanes).toEqual(profileSlots(cls))

      // A chain carried down from a wider window (longer than this row, and in another order) is
      // ignored rather than windowed onto the slots.
      useStore.getState().setWidthClass('ultra', true)
      useStore.setState((s) => ({
        layout: { ...s.layout, cascade: s.layout.slotPanes.slice(1).reverse() },
      }))
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

/**
 * 044 US2 (T021/T022): the push path. A drill-in appends a column, a click in a column that is not
 * the last truncates the chain, and a pane that leaves the chain leaves its object behind with it.
 */
describe('drilling in cascades to the right (044 US2)', () => {
  const pod = (name: string): ObjectRef => ({
    section: SECTION.pods,
    kind: 'Pod',
    namespace: 'default',
    name,
  })
  const targets = () => layout().slotPanes.map((id) => useStore.getState().panes[id]?.target?.name)

  it('appends a column per drill-in, leaving the one it came from (FR-005)', () => {
    useStore.getState().setWidthClass('ultra')
    const [, list] = layout().slotPanes
    useStore.getState().navigate(SECTION.pods)

    useStore.getState().openInspector(pod('a'), list)
    expect(layout().cascade).toHaveLength(2)
    expect(targets()).toEqual([undefined, undefined, 'a', undefined, undefined])

    // A related-resource link has no source pane: it drills in from the deepest step.
    useStore.getState().openInspector(pod('b'), undefined, true)
    useStore.getState().openInspector(pod('c'), undefined, true)
    expect(layout().cascade).toHaveLength(4)
    expect(targets()).toEqual([undefined, undefined, 'a', 'b', 'c'])
    expect(new Set(layout().cascade).size).toBe(4)
  })

  it('shifts the chain left when no empty column is left, keeping slots 0 and 1 (FR-006)', () => {
    useStore.getState().setWidthClass('wide')
    const [nav, list] = layout().slotPanes
    useStore.getState().navigate(SECTION.pods)
    for (const name of ['a', 'b']) useStore.getState().openInspector(pod(name), undefined, true)
    expect(targets()).toEqual([undefined, undefined, 'a', 'b'])

    useStore.getState().openInspector(pod('c'), undefined, true)
    expect(layout().slotPanes[0]).toBe(nav)
    expect(layout().slotPanes[1]).toBe(list)
    expect(targets()).toEqual([undefined, undefined, 'b', 'c'])
    // Nothing was lost: the widened window shows the step that scrolled off again (FR-010).
    useStore.getState().setWidthClass('ultra')
    expect(targets()).toEqual([undefined, undefined, 'a', 'b', 'c'])
  })

  it('truncates the chain when a row in an earlier column is picked (FR-007)', () => {
    useStore.getState().setWidthClass('ultra')
    const [, list] = layout().slotPanes
    useStore.getState().navigate(SECTION.pods)
    useStore.getState().openInspector(pod('a'), list)
    for (const name of ['b', 'c']) useStore.getState().openInspector(pod(name), undefined, true)
    const step1 = layout().slotPanes[2]

    useStore.getState().openInspector(pod('z'), list)
    expect(layout().cascade).toEqual([list, step1])
    // The same column takes the new object, and the columns to its right are empty again.
    expect(targets()).toEqual([undefined, undefined, 'z', undefined, undefined])
  })

  it('leaves no object behind in a column dropped from the chain (FR-002, FR-004)', () => {
    useStore.getState().setWidthClass('ultra')
    const [, list] = layout().slotPanes
    useStore.getState().navigate(SECTION.pods)
    useStore.getState().openInspector(pod('a'), list)
    useStore.getState().openInspector(pod('b'), undefined, true)

    // A section with nothing to auto-open must not still show the previous section's object.
    useStore.getState().navigate(SECTION.configmaps)
    expect(layout().cascade).toEqual([list])
    expect(targets()).toEqual([undefined, undefined, undefined, undefined, undefined])
    expect(
      Object.values(useStore.getState().panes).some(
        (p) => p.kind === 'inspector' && p.target !== undefined,
      ),
    ).toBe(false)
  })

  it('leaves no object behind on a context change (FR-015)', () => {
    useStore.getState().setWidthClass('ultra')
    const [, list] = layout().slotPanes
    useStore.getState().navigate(SECTION.pods)
    useStore.getState().openInspector(pod('a'), list)
    useStore.getState().openInspector(pod('b'), undefined, true)

    useStore.getState().selectContext('kind-other')
    expect(targets()).toEqual([undefined, undefined, undefined, undefined, undefined])
  })

  it.each(['tablet', 'laptop', 'desktop'] as const)(
    'keeps retargeting the one slotted inspector at %s (FR-009)',
    (cls) => {
      useStore.getState().setWidthClass(cls)
      const baseline = [...layout().slotPanes]
      const list = baseline.find((id) => kindOf(id) === 'list') ?? ''
      useStore.getState().openInspector(pod('a'), list)
      useStore.getState().openInspector(pod('b'), undefined, true)
      expect(layout().slotPanes).toEqual(baseline)
      const inspectors = baseline.filter((id) => kindOf(id) === 'inspector')
      expect(inspectors.map((id) => useStore.getState().panes[id]?.target?.name)).toEqual(['b'])
    },
  )
})
