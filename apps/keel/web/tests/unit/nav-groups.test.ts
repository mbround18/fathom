/** The grouped navigation's data (spec 063 T030; FR-001, FR-004). */
import { describe, expect, it } from 'vitest'

import { NAV_GROUPS, SECTIONS, visibleGroups, type NavGroup } from '@/app/nav'

import { ME } from './session-helpers'

const labels = (g: NavGroup) => g.sections.map((s) => s.label)

describe('nav groups', () => {
  it('has three groups in order, each with its sections in order', () => {
    expect(NAV_GROUPS.map((g) => [g.id, g.label, labels(g)])).toEqual([
      ['pipelines', 'Pipelines', ['Runs', 'Reviews', 'Bindings', 'Receivers']],
      ['infrastructure', 'Infrastructure', ['Inventory', 'Hosts']],
      ['administration', 'Administration', ['Teams', 'Users', 'Audit', 'Migration']],
    ])
  })

  it('derives SECTIONS as the flattened order', () => {
    expect(SECTIONS).toEqual(NAV_GROUPS.flatMap((g) => g.sections))
    expect(SECTIONS.map((s) => s.path)).toEqual([
      '/runs',
      '/reviews',
      '/bindings',
      '/receivers',
      '/inventory',
      '/hosts',
      '/teams',
      '/users',
      '/audit',
      '/migration',
    ])
  })

  it('shows every group and section while none is gated', () => {
    expect(visibleGroups(ME)).toEqual(NAV_GROUPS)
    expect(visibleGroups(null)).toEqual(NAV_GROUPS)
  })

  describe('with gated sections', () => {
    const [pipelines, infrastructure, administration] = NAV_GROUPS as [NavGroup, NavGroup, NavGroup]
    const ownersOnly = (me: { instance_role: string }) => me.instance_role === 'owner'
    const groups: NavGroup[] = [
      // Reviews is gated; the rest of Pipelines stays.
      { ...pipelines, sections: pipelines.sections.map((s) => (s.label === 'Reviews' ? { ...s, visible: ownersOnly } : s)) },
      // Every Infrastructure section is gated, so the group goes when they are hidden.
      { ...infrastructure, sections: infrastructure.sections.map((s) => ({ ...s, visible: ownersOnly })) },
      administration,
    ]

    it('drops a hidden section and a group left empty', () => {
      const seen = visibleGroups({ ...ME, instance_role: 'user' }, groups)
      expect(seen.map((g) => g.id)).toEqual(['pipelines', 'administration'])
      expect(seen[0]?.sections.map((s) => s.label)).toEqual(['Runs', 'Bindings', 'Receivers'])
    })

    it('keeps them when visible says yes', () => {
      const seen = visibleGroups({ ...ME, instance_role: 'owner' }, groups)
      expect(seen.map((g) => [g.id, labels(g)])).toEqual(NAV_GROUPS.map((g) => [g.id, labels(g)]))
    })

    it('hides gated sections while signed out', () => {
      expect(visibleGroups(null, groups).map((g) => g.id)).toEqual(['pipelines', 'administration'])
    })
  })
})
