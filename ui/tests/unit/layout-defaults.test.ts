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

  it('adds a sixth tool slot at ≥ 3840', () => {
    const p = defaultProfile('ultra', true)
    expect(p.slots).toHaveLength(6)
    expect(p.slots.at(-1)).toMatchObject({ kind: 'tool', tool: 'flux' })
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
