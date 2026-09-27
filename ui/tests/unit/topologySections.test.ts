/**
 * 043 US6 (FR-030/FR-032, edge case "deep link or saved route"): a saved slot pinned to a tool
 * that moved to the navigator opens its section; an unknown tool kind never throws; the
 * inspector keeps its selected tab across a "Follow" drill but not on a fresh open.
 */
import { describe, expect, it } from 'vitest'

import { resolveProfile } from '@/layout/defaults'
import type { LayoutProfile } from '@/protocol/types'
import type { ObjectRef } from '@/resources'
import { assignPanes, MOVED_TOOL_SECTIONS, useStore } from '@/store'

describe('saved slots pinned to a moved dock tool', () => {
  it('open the topology section in a list pane instead of a tool pane', () => {
    const profile: LayoutProfile = { slots: [{ kind: 'tool', tool: 'topology', sizePct: 100 }] }
    const { slotPanes, panes } = assignPanes(profile, {})
    const pane = panes[slotPanes[0] ?? '']
    expect(pane?.kind).toBe('list')
    expect(pane?.route?.section).toBe('topology')
  })

  it('never take over another list pane and its route', () => {
    const lists: LayoutProfile = { slots: [{ kind: 'list', sizePct: 100 }] }
    const first = assignPanes(lists, {})
    const listId = first.slotPanes[0] ?? ''
    const profile: LayoutProfile = { slots: [{ kind: 'tool', tool: 'topology', sizePct: 100 }] }
    const { slotPanes, panes } = assignPanes(profile, first.panes)
    expect(slotPanes[0]).not.toBe(listId)
    expect(panes[slotPanes[0] ?? '']?.route?.section).toBe('topology')
  })

  it('map every moved tool to its navigator section', () => {
    expect(MOVED_TOOL_SECTIONS.topology).toBe('topology')
    expect(MOVED_TOOL_SECTIONS.linter).toBe('linter')
    expect(MOVED_TOOL_SECTIONS.logs).toBeUndefined()
  })

  it('leave a still-docked tool on its tool pane', () => {
    const profile: LayoutProfile = { slots: [{ kind: 'tool', tool: 'logs', sizePct: 100 }] }
    const { slotPanes, panes } = assignPanes(profile, {})
    expect(panes[slotPanes[0] ?? '']?.kind).toBe('tool')
  })

  it('fall back to the default layout for an unknown tool kind, without throwing', () => {
    const saved = {
      slots: [{ kind: 'tool', tool: 'no-such-tool', sizePct: 100 }],
    } as unknown as LayoutProfile
    expect(() => resolveProfile('desktop', saved)).not.toThrow()
    expect(resolveProfile('desktop', saved)).toEqual(resolveProfile('desktop', null))
  })
})

describe('inspector tab across Follow', () => {
  const deploy: ObjectRef = {
    section: 'workloads/deployments',
    kind: 'Deployment',
    namespace: 'shop',
    name: 'web',
  }
  const rs: ObjectRef = {
    section: 'workloads/replicasets',
    kind: 'ReplicaSet',
    namespace: 'shop',
    name: 'web-7d9',
  }

  function inspectorPane() {
    const panes = Object.values(useStore.getState().panes)
    return panes.find((p) => p.kind === 'inspector' && p.target !== undefined)
  }

  it('keeps the Topology tab on a drill and resets it on a fresh open', () => {
    const { openInspector, updatePane } = useStore.getState()
    openInspector(deploy)
    const first = inspectorPane()
    expect(first?.inspectorTab).toBeUndefined()
    updatePane(first?.id ?? '', { inspectorTab: 'topology' })

    openInspector(rs, undefined, true)
    const drilled = inspectorPane()
    expect(drilled?.target?.name).toBe('web-7d9')
    expect(drilled?.inspectorTab).toBe('topology')
    expect(drilled?.history?.map((r) => r.name)).toEqual(['web'])

    openInspector(deploy)
    expect(inspectorPane()?.inspectorTab).toBeUndefined()
  })
})
