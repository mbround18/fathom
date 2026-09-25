import { beforeEach, describe, expect, it } from 'vitest'

import { defaultProfile, resolveProfile } from '@/layout/defaults'
import { visibleSlots, WIDTH_CLASSES } from '@/layout/width-class'
import type { LayoutProfile } from '@/protocol/types'
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
    expect(defaultProfile('desktop').dock).toMatchObject({
      open: true,
      tabs: ['logs', 'events', 'topology', 'daemon', 'velero', 'certManager', 'knative', 'keda', 'linter'],
    })
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
