import { useEffect, useSyncExternalStore } from 'react'

import { useStore } from '@/store'

import {
  BREAKPOINT_QUERIES,
  densityFor,
  visibleSlots,
  WIDTH_CLASSES,
  type WidthClass,
} from './width-class'

export interface WidthClassInfo {
  widthClass: WidthClass
  /** Ultra at ≥ 3840 px: one extra slot. */
  ultraPlus: boolean
  slots: number
}

/**
 * Snapshot = how many breakpoint queries match (0..6). Media queries are nested min-widths, so
 * the count identifies the class; a primitive keeps useSyncExternalStore stable.
 */
function readLevel(): number {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return 0
  let level = 0
  for (const q of BREAKPOINT_QUERIES) {
    if (window.matchMedia(q).matches) level++
  }
  return level
}

function subscribe(onChange: () => void): () => void {
  if (typeof window === 'undefined' || typeof window.matchMedia !== 'function') return () => {}
  const lists = BREAKPOINT_QUERIES.map((q) => window.matchMedia(q))
  for (const mql of lists) mql.addEventListener('change', onChange)
  return () => {
    for (const mql of lists) mql.removeEventListener('change', onChange)
  }
}

function infoFromLevel(level: number): WidthClassInfo {
  const ultraPlus = level > WIDTH_CLASSES.length - 1
  const widthClass = WIDTH_CLASSES[Math.min(level, WIDTH_CLASSES.length - 1)] ?? 'phone'
  // Any px ≥ 3840 selects the sub-step; the exact value is irrelevant to the slot count.
  return { widthClass, ultraPlus, slots: visibleSlots(widthClass, ultraPlus ? Infinity : 0) }
}

/**
 * Tracks the width class with `matchMedia` listeners (no resize polling), and mirrors it to
 * `<html data-width-class>`, `store.layout`, and the density CSS variables.
 */
export function useWidthClass(): WidthClassInfo {
  const level = useSyncExternalStore(subscribe, readLevel, () => 0)
  const info = infoFromLevel(level)
  const setWidthClass = useStore((s) => s.setWidthClass)
  const override = useStore((s) => s.ui.density.override)

  useEffect(() => {
    const root = document.documentElement
    const d = densityFor(info.widthClass, override ?? undefined)
    root.dataset.widthClass = info.widthClass
    root.style.setProperty('--row-height', `${d.rowHeight}px`)
    root.style.setProperty('--touch-target', `${d.touchTarget}px`)
    root.style.setProperty('--font-size-base', `${d.fontSize}px`)
    setWidthClass(info.widthClass, info.ultraPlus)
  }, [info.widthClass, info.ultraPlus, override, setWidthClass])

  return info
}
