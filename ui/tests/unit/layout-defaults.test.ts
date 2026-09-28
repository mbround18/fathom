import { beforeEach, describe, expect, it } from 'vitest'

import {
  DOCK_ALLOWED,
  defaultProfile,
  dockTabs,
  filterDock,
  resolveProfile,
} from '@/layout/defaults'
import { visibleSlots, WIDTH_CLASSES } from '@/layout/width-class'
import type { DockConfig, LayoutProfile, ToolKind } from '@/protocol/types'
import { useStore } from '@/store'

const sum = (p: LayoutProfile) => p.slots.reduce((t, s) => t + s.sizePct, 0)

describe('defaultProfile', () => {
  it.each(WIDTH_CLASSES)('%s fills exactly visibleSlots and sums to 100', (cls) => {
    const p = defaultProfile(cls)
    expect(p.slots).toHaveLength(visibleSlots(cls))
    expect(sum(p)).toBeCloseTo(100, 1)
  })

  it('adds a sixth cascade slot at ≥ 3840 (044)', () => {
    const p = defaultProfile('ultra', true)
    expect(p.slots).toHaveLength(6)
    expect(p.slots.at(-1)).toMatchObject({ kind: 'inspector' })
    expect(p.slots.at(-1)).not.toHaveProperty('tool')
  })

  it('has a dock from laptop up only', () => {
    expect(defaultProfile('tablet').dock).toBeUndefined()
    expect(defaultProfile('laptop').dock).toBeDefined()
    expect(defaultProfile('desktop').dock).toEqual({
      open: true,
      heightPct: 30,
      tabs: ['logs', 'events'],
    })
  })
})

/** The dock every build before 043 wrote by default. */
const OLD_NINE: ToolKind[] = [
  'logs',
  'events',
  'topology',
  'daemon',
  'velero',
  'certManager',
  'knative',
  'keda',
  'linter',
]

describe('dock policy (043)', () => {
  it('accepts only the streams', () => {
    expect(DOCK_ALLOWED).toEqual(['logs', 'events', 'daemon'])
  })

  it('filters the old nine-tab dock to logs, events and daemon, keeping height and state', () => {
    const saved: DockConfig = { open: false, heightPct: 42, tabs: OLD_NINE }
    expect(filterDock(saved)).toEqual({
      open: false,
      heightPct: 42,
      tabs: ['logs', 'events', 'daemon'],
    })
    expect(saved.tabs).toHaveLength(9)
  })

  it('keeps the saved order of the allowed tabs', () => {
    const saved: DockConfig = {
      open: true,
      heightPct: 25,
      tabs: ['linter', 'events', 'topology', 'logs'],
    }
    expect(filterDock(saved).tabs).toEqual(['events', 'logs'])
  })

  it('renders the old nine-tab dock as Logs and Events when the daemon is unavailable', () => {
    expect(dockTabs({ open: true, heightPct: 30, tabs: OLD_NINE }, false)).toEqual([
      'logs',
      'events',
    ])
  })

  it('adds Cluster history while the daemon is available and removes it otherwise', () => {
    const dock: DockConfig = { open: true, heightPct: 30, tabs: ['logs', 'events'] }
    expect(dockTabs(dock, true)).toEqual(['logs', 'events', 'daemon'])
    expect(dockTabs(dock, false)).toEqual(['logs', 'events'])
    const withDaemon: DockConfig = { ...dock, tabs: ['daemon', 'logs', 'events'] }
    expect(dockTabs(withDaemon, true)).toEqual(['daemon', 'logs', 'events'])
    expect(dockTabs(withDaemon, false)).toEqual(['logs', 'events'])
    expect(dock.tabs).toEqual(['logs', 'events'])
  })

  it('applies the filter when a saved profile loads, keeping height and open state', () => {
    const saved: LayoutProfile = {
      slots: defaultProfile('desktop').slots,
      dock: { open: false, heightPct: 44, tabs: OLD_NINE },
    }
    expect(resolveProfile('desktop', saved).dock).toEqual({
      open: false,
      heightPct: 44,
      tabs: ['logs', 'events', 'daemon'],
    })
    expect(saved.dock?.tabs).toHaveLength(9)
  })
})

describe('cascade profiles (044)', () => {
  const kinds = (p: LayoutProfile) => p.slots.map((s) => s.kind)

  it.each(['wide', 'ultra'] as const)('%s opens with one list and no tool slot', (cls) => {
    const p = defaultProfile(cls)
    expect(kinds(p)).toEqual(['nav', 'list', ...p.slots.slice(2).map(() => 'inspector')])
    expect(kinds(p).filter((k) => k === 'list')).toHaveLength(1)
    expect(kinds(p)).not.toContain('tool')
    expect(p.slots.every((s) => s.tool === undefined)).toBe(true)
  })

  it('keeps the slot count of every class (Principle IV, FR-008)', () => {
    for (const cls of WIDTH_CLASSES)
      expect(defaultProfile(cls).slots).toHaveLength(visibleSlots(cls))
    expect(defaultProfile('ultra', true).slots).toHaveLength(visibleSlots('ultra', Infinity))
  })

  it('resolves a saved wide profile with a second list to cascade slots, keeping its sizes', () => {
    const saved: LayoutProfile = {
      slots: [
        { kind: 'nav', sizePct: 10 },
        { kind: 'list', sizePct: 40 },
        { kind: 'list', sizePct: 30 },
        { kind: 'inspector', sizePct: 20 },
      ],
    }
    const p = resolveProfile('wide', saved)
    expect(kinds(p)).toEqual(['nav', 'list', 'inspector', 'inspector'])
    expect(p.slots.map((s) => s.sizePct)).toEqual([10, 40, 30, 20])
    expect(sum(p)).toBeCloseTo(100, 1)
  })

  it('normalises a saved ultra profile that pinned Overview and Flux (FR-011)', () => {
    const saved: LayoutProfile = {
      slots: [
        { kind: 'nav', sizePct: 10 },
        { kind: 'list', sizePct: 24 },
        { kind: 'list', sizePct: 24 },
        { kind: 'inspector', sizePct: 22 },
        { kind: 'tool', sizePct: 20, tool: 'overview' },
        { kind: 'tool', sizePct: 16, tool: 'flux' },
      ],
    }
    const p = resolveProfile('ultra', saved, true)
    expect(kinds(p)).toEqual(['nav', 'list', 'inspector', 'inspector', 'inspector', 'inspector'])
    expect(p.slots.every((s) => s.tool === undefined)).toBe(true)
    expect(sum(p)).toBeCloseTo(100, 1)
    // Truncated to five slots below 3840, still with no tool slot left.
    expect(kinds(resolveProfile('ultra', saved))).toEqual([
      'nav',
      'list',
      'inspector',
      'inspector',
      'inspector',
    ])
  })

  it('still falls back to the default for an invalid saved wide profile', () => {
    const bad = { slots: [{ kind: 'nope', sizePct: 50 }] } as unknown as LayoutProfile
    expect(resolveProfile('wide', bad)).toEqual(defaultProfile('wide'))
    expect(resolveProfile('ultra', { slots: [] })).toEqual(defaultProfile('ultra'))
  })

  it.each(['phone', 'tablet', 'laptop', 'desktop'] as const)(
    'leaves a saved %s profile untouched (FR-009)',
    (cls) => {
      const saved: LayoutProfile = {
        slots: defaultProfile(cls).slots.map((s, i) =>
          i === 0 ? { ...s, kind: 'tool', tool: 'overview' } : s,
        ),
        ...(defaultProfile(cls).dock
          ? { dock: { open: true, heightPct: 30, tabs: ['logs'] } }
          : {}),
      }
      const p = resolveProfile(cls, saved)
      expect(kinds(p)).toEqual(kinds(saved))
      expect(p.slots[0]?.tool).toBe('overview')
    },
  )
})

describe('resolveProfile', () => {
  it('falls back to the default for missing or malformed profiles', () => {
    expect(resolveProfile('laptop')).toEqual(defaultProfile('laptop'))
    const bad = { slots: [{ kind: 'bogus', sizePct: 50 }] } as unknown as LayoutProfile
    expect(resolveProfile('laptop', bad)).toEqual(defaultProfile('laptop'))
  })

  it('truncates saved slots to visibleSlots and renormalizes', () => {
    const saved = defaultProfile('ultra')
    const p = resolveProfile('tablet', saved)
    expect(p.slots).toHaveLength(2)
    expect(sum(p)).toBeCloseTo(100, 1)
    expect(p.dock).toBeUndefined()
  })

  it('keeps a valid saved profile', () => {
    const saved: LayoutProfile = {
      slots: [
        { kind: 'nav', sizePct: 20 },
        { kind: 'list', sizePct: 80 },
      ],
    }
    expect(resolveProfile('laptop', saved).slots).toEqual(saved.slots)
  })
})

describe('store layout', () => {
  beforeEach(() => useStore.getState().setWidthClass('phone'))

  it('keeps pane state across narrow → wide → narrow (FR-007)', () => {
    const s = useStore.getState()
    s.setWidthClass('ultra')
    const second = useStore.getState().layout.slotPanes[2]
    expect(second).toBeDefined()
    useStore.getState().updatePane(second ?? '', { search: 'nginx', scrollTop: 420 })

    useStore.getState().setWidthClass('phone')
    expect(useStore.getState().layout.slotPanes).toHaveLength(1)
    expect(useStore.getState().panes[second ?? '']).toMatchObject({ search: 'nginx' })

    useStore.getState().setWidthClass('ultra')
    const restored = useStore.getState().layout.slotPanes[2]
    expect(restored).toBe(second)
    expect(useStore.getState().panes[restored ?? '']).toMatchObject({ scrollTop: 420 })
  })

  it('gives every visible slot a distinct pane', () => {
    useStore.getState().setWidthClass('ultra', true)
    const ids = useStore.getState().layout.slotPanes
    expect(new Set(ids).size).toBe(6)
  })
})
