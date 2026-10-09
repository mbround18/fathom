/**
 * Tablet inspector targeting. The tablet content area is either the Resources row (list +
 * inspector slots) or one full-width tool tab, never both. An open while the row is on screen,
 * from any pane in it and whether or not it names a `from`, must land in the slotted inspector;
 * an open while a tool tab is active must push onto the stack, which TabletTabs renders with Back.
 */
import { beforeEach, describe, expect, it } from 'vitest'

import { SECTION } from '@/protocol/commands'
import type { ObjectRef } from '@/resources/types'
import { initialData, useStore, type PaneId } from '@/store'

const state = () => useStore.getState()
const layout = () => state().layout
const slottedInspector = (): PaneId =>
  layout().slotPanes.find((id) => state().panes[id]?.kind === 'inspector') ?? ''
const stackedInspectors = (): PaneId[] =>
  layout().stack.filter((id) => state().panes[id]?.kind === 'inspector')

const FLUX_REF: ObjectRef = {
  section: 'integrations/flux-resource',
  kind: 'Kustomization',
  namespace: 'flux-system',
  name: 'apps',
}

beforeEach(() => {
  useStore.setState({ ...initialData(), send: () => undefined })
})

describe('tablet inspector targeting', () => {
  it('retargets the slotted inspector from a pane in the visible row, even without `from`', () => {
    state().setWidthClass('tablet')
    state().navigate(SECTION.flux)
    state().openInspector(FLUX_REF)
    expect(state().panes[slottedInspector()]?.target).toEqual(FLUX_REF)
    expect(stackedInspectors()).toEqual([])
  })

  it('pushes onto the stack while a tool tab hides the row', () => {
    state().setWidthClass('tablet')
    state().openRoot('tool', 'topology')
    const slotted = slottedInspector()
    state().openInspector(FLUX_REF)
    expect(state().panes[slotted]?.target).toBeUndefined()
    const [pushed] = stackedInspectors()
    expect(pushed).toBeDefined()
    expect(state().panes[pushed ?? '']?.target).toEqual(FLUX_REF)
    expect(layout().stack.at(-1)).toBe(pushed)
  })

  it('still drills in on phone', () => {
    state().setWidthClass('phone')
    state().navigate(SECTION.flux)
    state().openInspector(FLUX_REF)
    const [pushed] = stackedInspectors()
    expect(state().panes[pushed ?? '']?.target).toEqual(FLUX_REF)
  })
})
