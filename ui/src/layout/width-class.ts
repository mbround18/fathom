/**
 * Width classes, slot counts, and density (contracts/layout.md, research R5, FR-006/FR-012).
 * Pure functions only; the React hook lives in ./use-width-class.ts.
 */
import type { WidthClass } from '@/protocol/types'

export type { WidthClass }

/** Ascending order. Every consumer that iterates classes relies on this order. */
export const WIDTH_CLASSES = [
  'phone',
  'tablet',
  'laptop',
  'desktop',
  'wide',
  'ultra',
] as const satisfies readonly WidthClass[]

/** Minimum logical viewport width (CSS px) for each class. */
export const MIN_WIDTH: Readonly<Record<WidthClass, number>> = {
  phone: 0,
  tablet: 640,
  laptop: 1024,
  desktop: 1440,
  wide: 1920,
  ultra: 2560,
}

/** At ≥ 3840 px an ultra layout gains a 6th slot. */
export const ULTRA_PLUS_MIN_WIDTH = 3840

const SLOTS: Readonly<Record<WidthClass, number>> = {
  phone: 1,
  tablet: 2,
  laptop: 3,
  desktop: 3,
  wide: 4,
  ultra: 5,
}

export function widthClassOf(px: number): WidthClass {
  let cls: WidthClass = 'phone'
  for (const c of WIDTH_CLASSES) {
    if (px >= MIN_WIDTH[c]) cls = c
  }
  return cls
}

/**
 * Number of pane slots visible side by side. Monotonic in width (FR-006): never decreases as
 * the class, or the ultra sub-step, goes up. The dock is not a slot; see {@link hasDock}.
 */
export function visibleSlots(cls: WidthClass, px?: number): number {
  const base = SLOTS[cls]
  return cls === 'ultra' && px !== undefined && px >= ULTRA_PLUS_MIN_WIDTH ? base + 1 : base
}

/** The bottom logs/events dock exists from desktop up. */
export function hasDock(cls: WidthClass): boolean {
  return MIN_WIDTH[cls] >= MIN_WIDTH.desktop
}

export interface Density {
  /** Table/list row height, px. */
  rowHeight: number
  /** Minimum interactive target, px. */
  touchTarget: number
  /** Base body font size, px. */
  fontSize: number
}

export const DENSITY: Readonly<Record<WidthClass, Density>> = {
  phone: { rowHeight: 44, touchTarget: 44, fontSize: 15 },
  tablet: { rowHeight: 44, touchTarget: 44, fontSize: 15 },
  laptop: { rowHeight: 28, touchTarget: 24, fontSize: 13 },
  desktop: { rowHeight: 28, touchTarget: 24, fontSize: 13 },
  wide: { rowHeight: 24, touchTarget: 24, fontSize: 13 },
  ultra: { rowHeight: 24, touchTarget: 24, fontSize: 13 },
}

/** Media queries whose match count identifies the class (and the ultra sub-step). */
export const BREAKPOINT_QUERIES: readonly string[] = [
  ...WIDTH_CLASSES.slice(1).map((c) => `(min-width: ${MIN_WIDTH[c]}px)`),
  `(min-width: ${ULTRA_PLUS_MIN_WIDTH}px)`,
]
