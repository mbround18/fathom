import { act, renderHook } from '@testing-library/react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'

import { useWidthClass } from '@/layout/use-width-class'
import {
  DENSITY,
  hasDock,
  ULTRA_PLUS_MIN_WIDTH,
  visibleSlots,
  WIDTH_CLASSES,
  widthClassOf,
} from '@/layout/width-class'
import { useStore } from '@/store'

describe('widthClassOf (contracts/layout.md)', () => {
  it.each([
    [0, 'phone'],
    [360, 'phone'],
    [390, 'phone'],
    [639, 'phone'],
    [639.5, 'phone'],
    [640, 'tablet'],
    [1023, 'tablet'],
    [1024, 'laptop'],
    [1439, 'laptop'],
    [1440, 'desktop'],
    [1919, 'desktop'],
    [1920, 'wide'],
    [2559, 'wide'],
    [2560, 'ultra'],
    [3839, 'ultra'],
    [3840, 'ultra'],
    [5120, 'ultra'],
  ] as const)('%d px → %s', (px, cls) => {
    expect(widthClassOf(px)).toBe(cls)
  })
})

describe('visibleSlots (FR-006)', () => {
  it('matches the contract table', () => {
    expect(visibleSlots('phone')).toBe(1)
    expect(visibleSlots('tablet')).toBe(2)
    expect(visibleSlots('laptop')).toBe(3)
    expect(visibleSlots('desktop')).toBe(3)
    expect(visibleSlots('wide')).toBe(4)
    expect(visibleSlots('ultra')).toBe(5)
    expect(visibleSlots('ultra', ULTRA_PLUS_MIN_WIDTH - 1)).toBe(5)
    expect(visibleSlots('ultra', ULTRA_PLUS_MIN_WIDTH)).toBe(6)
  })

  it('never decreases as the class goes up, including the ≥3840 sub-step', () => {
    const steps = [
      ...WIDTH_CLASSES.map((cls) => visibleSlots(cls)),
      visibleSlots('ultra', ULTRA_PLUS_MIN_WIDTH),
    ]
    for (let i = 1; i < steps.length; i++) {
      expect(steps[i]).toBeGreaterThanOrEqual(steps[i - 1] ?? 0)
    }
  })

  it('never decreases across every pixel width from 0 to 6000', () => {
    let prev = 0
    for (let px = 0; px <= 6000; px++) {
      const slots = visibleSlots(widthClassOf(px), px)
      expect(slots).toBeGreaterThanOrEqual(prev)
      prev = slots
    }
  })

  it('the sub-step only applies to ultra', () => {
    expect(visibleSlots('wide', 5000)).toBe(4)
  })

  it('adds the dock from desktop up', () => {
    expect(WIDTH_CLASSES.map(hasDock)).toEqual([false, false, false, true, true, true])
  })
})

describe('DENSITY (contracts/layout.md § Density)', () => {
  it('uses touch rows on phone/tablet, compact from laptop, densest from wide', () => {
    expect(DENSITY.phone).toMatchObject({ rowHeight: 44, touchTarget: 44, fontSize: 15 })
    expect(DENSITY.tablet).toMatchObject({ rowHeight: 44, touchTarget: 44, fontSize: 15 })
    expect(DENSITY.laptop).toMatchObject({ rowHeight: 28, fontSize: 13 })
    expect(DENSITY.desktop).toMatchObject({ rowHeight: 28, fontSize: 13 })
    expect(DENSITY.wide).toMatchObject({ rowHeight: 24, fontSize: 13 })
    expect(DENSITY.ultra).toMatchObject({ rowHeight: 24, fontSize: 13 })
  })
})

describe('useWidthClass', () => {
  let width = 390
  const listeners = new Set<() => void>()

  beforeEach(() => {
    width = 390
    listeners.clear()
    window.matchMedia = (query: string): MediaQueryList => {
      const min = Number(/min-width:\s*(\d+)px/.exec(query)?.[1] ?? 0)
      return {
        get matches() {
          return width >= min
        },
        media: query,
        onchange: null,
        addEventListener: (_: string, cb: EventListenerOrEventListenerObject) => {
          listeners.add(cb as () => void)
        },
        removeEventListener: (_: string, cb: EventListenerOrEventListenerObject) => {
          listeners.delete(cb as () => void)
        },
        addListener: () => undefined,
        removeListener: () => undefined,
        dispatchEvent: () => false,
      }
    }
  })

  afterEach(() => {
    delete document.documentElement.dataset.widthClass
  })

  const resize = (px: number) => {
    width = px
    act(() => {
      for (const cb of listeners) cb()
    })
  }

  it('tracks media-query changes into <html>, the store, and density variables', () => {
    const { result } = renderHook(() => useWidthClass())
    expect(result.current.widthClass).toBe('phone')
    expect(document.documentElement.dataset.widthClass).toBe('phone')
    expect(useStore.getState().layout.widthClass).toBe('phone')
    expect(document.documentElement.style.getPropertyValue('--row-height')).toBe('44px')

    resize(2560)
    expect(result.current).toEqual({ widthClass: 'ultra', ultraPlus: false, slots: 5 })
    expect(document.documentElement.dataset.widthClass).toBe('ultra')
    expect(useStore.getState().layout.widthClass).toBe('ultra')
    expect(document.documentElement.style.getPropertyValue('--row-height')).toBe('24px')

    resize(5120)
    expect(result.current).toEqual({ widthClass: 'ultra', ultraPlus: true, slots: 6 })

    resize(1280)
    expect(result.current.widthClass).toBe('laptop')
    expect(document.documentElement.style.getPropertyValue('--font-size-base')).toBe('13px')
  })
})
