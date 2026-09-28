/**
 * 044 US1 (T015): the arm/consume/clear rules of the section auto-open (FR-003, FR-009, FR-015).
 * The scan itself is `mostRelevantRow` (tests/unit/relevance.test.ts); what is tested here is when
 * it may run and whose selection wins when both the operator and a slow list want the column.
 */
import { beforeEach, describe, expect, it } from 'vitest'

import { SECTION } from '@/protocol/commands'
import type { ObjectRef } from '@/resources/types'
import { initialData, useStore, type PaneId } from '@/store'

const layout = () => useStore.getState().layout
const listPane = (): PaneId => layout().cascade[0] ?? ''

const POD: ObjectRef = {
  section: SECTION.pods,
  kind: 'Pod',
  namespace: 'default',
  name: 'api-1',
}
const OTHER_POD: ObjectRef = { ...POD, name: 'api-2' }

beforeEach(() => {
  useStore.setState({ ...initialData(), send: () => undefined })
})

describe('section auto-open (044 US1)', () => {
  it.each(['wide', 'ultra'] as const)('arms on a section click at %s (FR-003)', (cls) => {
    useStore.getState().setWidthClass(cls)
    useStore.getState().navigate(SECTION.pods)
    expect(layout().pendingAutoOpen?.paneId).toBe(listPane())
    expect(layout().pendingAutoOpen?.route.section).toBe(SECTION.pods)
  })

  it.each(['phone', 'tablet', 'laptop', 'desktop'] as const)(
    'arms nothing at %s width (FR-009)',
    (cls) => {
      useStore.getState().setWidthClass(cls)
      useStore.getState().navigate(SECTION.pods)
      expect(layout().pendingAutoOpen).toBeUndefined()
    },
  )

  it('opens the chosen row once and marks the step (contract "Slots and test ids")', () => {
    useStore.getState().setWidthClass('wide')
    useStore.getState().navigate(SECTION.pods)
    const list = listPane()
    useStore.getState().autoOpenStep(list, POD)

    const step = layout().slotPanes[2]
    expect(layout().autoOpened).toBe(step)
    expect(useStore.getState().panes[step ?? '']?.target).toEqual(POD)
    // Consumed: a second, later scan of the same list cannot reopen or retarget the column.
    expect(layout().pendingAutoOpen).toBeUndefined()
    useStore.getState().autoOpenStep(list, OTHER_POD)
    expect(useStore.getState().panes[step ?? '']?.target).toEqual(POD)
  })

  it('leaves the same trail as a row click: the row is selected in the list', () => {
    useStore.getState().setWidthClass('wide')
    useStore.getState().navigate(SECTION.pods)
    useStore.getState().autoOpenStep(listPane(), POD)
    expect(useStore.getState().panes[listPane()]).toMatchObject({
      selection: 'default/api-1',
      focusedRow: 'default/api-1',
    })
  })

  it('never fires for a pane the arm does not name', () => {
    useStore.getState().setWidthClass('wide')
    useStore.getState().navigate(SECTION.pods)
    const other = layout().slotPanes[2] ?? ''
    useStore.getState().autoOpenStep(other, POD)
    expect(layout().pendingAutoOpen).not.toBeUndefined()
    expect(layout().autoOpened).toBeUndefined()
  })

  describe('a superseding operator action clears the arm (T013)', () => {
    const arm = () => {
      useStore.getState().setWidthClass('wide')
      useStore.getState().navigate(SECTION.pods)
      expect(layout().pendingAutoOpen).not.toBeUndefined()
    }

    it('a row click: the operator’s own choice keeps the column', () => {
      arm()
      const list = listPane()
      useStore.getState().openInspector(OTHER_POD, list)
      expect(layout().pendingAutoOpen).toBeUndefined()
      // A late scan of the list must not replace what they opened.
      useStore.getState().autoOpenStep(list, POD)
      expect(useStore.getState().panes[layout().slotPanes[2] ?? '']?.target).toEqual(OTHER_POD)
      expect(layout().autoOpened).toBeUndefined()
    })

    it('a section change: the arm names the new section', () => {
      arm()
      useStore.getState().navigate(SECTION.deployments)
      expect(layout().pendingAutoOpen?.route.section).toBe(SECTION.deployments)
      // A pods scan landing late is for the right pane but the wrong section: it opens nothing,
      // and the deployments arm it must not consume is still there.
      useStore.getState().autoOpenStep(listPane(), POD)
      expect(layout().autoOpened).toBeUndefined()
      expect(layout().pendingAutoOpen?.route.section).toBe(SECTION.deployments)
    })

    it('a context change (FR-015)', () => {
      arm()
      useStore.getState().selectContext('kind-other')
      expect(layout().pendingAutoOpen).toBeUndefined()
      expect(layout().autoOpened).toBeUndefined()
    })

    it('dropping below wide width (FR-009)', () => {
      arm()
      useStore.getState().setWidthClass('laptop')
      expect(layout().pendingAutoOpen).toBeUndefined()
      // Growing back does not resurrect it: the operator clicks a section again.
      useStore.getState().setWidthClass('ultra')
      expect(layout().pendingAutoOpen).toBeUndefined()
    })

    it('a click anywhere in the cascade drops the marker', () => {
      arm()
      useStore.getState().autoOpenStep(listPane(), POD)
      expect(layout().autoOpened).not.toBeUndefined()
      useStore.getState().clearAutoOpen()
      expect(layout().autoOpened).toBeUndefined()
      expect(useStore.getState().panes[layout().slotPanes[2] ?? '']?.target).toEqual(POD)
    })
  })

  it('a width change inside the cascade classes keeps the arm alive (FR-010)', () => {
    useStore.getState().setWidthClass('wide')
    useStore.getState().navigate(SECTION.pods)
    useStore.getState().setWidthClass('ultra')
    expect(layout().pendingAutoOpen?.paneId).toBe(listPane())
  })

  it('scans nothing before the first page: the arm alone opens no column', () => {
    useStore.getState().setWidthClass('wide')
    useStore.getState().navigate(SECTION.pods)
    // Armed, but no list state has arrived, so no step exists and no read was issued.
    expect(layout().autoOpened).toBeUndefined()
    expect(
      layout()
        .slotPanes.slice(2)
        .map((id) => useStore.getState().panes[id]?.target),
    ).toEqual([undefined, undefined])
  })
})
