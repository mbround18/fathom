/**
 * Pure viewport math + hit-testing for `TopologyCanvas` (T040). Kept separate from the
 * component so it is unit-testable without mounting a `<canvas>`.
 */
import { NODE_RADIUS } from '@/topology/layout'
import type { TopologyNode } from '@/topology/types'

export interface Camera {
  scale: number
  /** World-space point currently centered under the canvas's top-left origin, in screen px. */
  x: number
  y: number
}

/** Screen (canvas-local, pointer event) coordinates -> world (graph/node) coordinates. */
export function screenToWorld(camera: Camera, sx: number, sy: number): { x: number; y: number } {
  return { x: (sx - camera.x) / camera.scale, y: (sy - camera.y) / camera.scale }
}

/** World coordinates -> screen coordinates, the inverse of {@link screenToWorld}. */
export function worldToScreen(camera: Camera, wx: number, wy: number): { x: number; y: number } {
  return { x: wx * camera.scale + camera.x, y: wy * camera.scale + camera.y }
}

/**
 * Zoom by `factor` (>1 = in, <1 = out) about the screen point `(sx, sy)`, so the world point
 * currently under the cursor/pinch-center stays under it after the zoom (research.md R4's
 * viewport model — scale + translation, no separate "zoom origin" state).
 */
export function zoomAt(camera: Camera, factor: number, sx: number, sy: number): Camera {
  const scale = clampScale(camera.scale * factor)
  const appliedFactor = scale / camera.scale
  return {
    scale,
    x: sx - (sx - camera.x) * appliedFactor,
    y: sy - (sy - camera.y) * appliedFactor,
  }
}

/** Low enough that "Fit" can show a whole all-namespaces graph on a phone. */
export const MIN_SCALE = 0.05
export const MAX_SCALE = 8

export function clampScale(scale: number): number {
  return Math.min(MAX_SCALE, Math.max(MIN_SCALE, scale))
}

/** The topmost node whose radius contains the world point, or `undefined` (nothing hit). */
export function hitTest(
  world: { x: number; y: number },
  nodes: readonly TopologyNode[],
): TopologyNode | undefined {
  for (let i = nodes.length - 1; i >= 0; i--) {
    const n = nodes[i]
    if (n?.x === undefined || n.y === undefined) continue
    const r = NODE_RADIUS[n.tier]
    const dx = world.x - n.x
    const dy = world.y - n.y
    if (dx * dx + dy * dy <= r * r) return n
  }
  return undefined
}

/**
 * A theme token's value as a canvas color. Tokens hold complete colors (`oklch(51% 0.11 163)`,
 * `#39ffb0`); only a bare channel triple (`51% 0.11 163`) needs wrapping. Wrapping a complete
 * color gives `oklch(oklch(...))`, which canvas rejects, silently keeping its default black fill.
 */
export function tokenColor(raw: string, fallback: string): string {
  const v = raw.trim()
  if (!v) return fallback
  return v.includes('(') || v.startsWith('#') ? v : `oklch(${v})`
}

/** Largest scale auto-fit may choose: below namespace focus, so fitting never changes focus. */
export const FIT_MAX_SCALE = 2

/**
 * The camera that fits every positioned node (plus its radius and a label line) into a
 * `width` x `height` viewport, centered, with `padding` px on each side. Scale is capped at
 * {@link FIT_MAX_SCALE} so a sparse graph is not blown up past the focus thresholds.
 */
export function fitCamera(
  nodes: readonly TopologyNode[],
  width: number,
  height: number,
  padding = 24,
): Camera {
  let minX = Infinity
  let minY = Infinity
  let maxX = -Infinity
  let maxY = -Infinity
  for (const n of nodes) {
    if (n.x === undefined || n.y === undefined) continue
    const r = NODE_RADIUS[n.tier]
    minX = Math.min(minX, n.x - r)
    maxX = Math.max(maxX, n.x + r)
    minY = Math.min(minY, n.y - r)
    maxY = Math.max(maxY, n.y + r + 14) // label line under the node
  }
  if (!Number.isFinite(minX)) return { scale: 1, x: 0, y: 0 }
  const bw = Math.max(1, maxX - minX)
  const bh = Math.max(1, maxY - minY)
  const availW = Math.max(1, width - padding * 2)
  const availH = Math.max(1, height - padding * 2)
  const scale = clampScale(Math.min(FIT_MAX_SCALE, availW / bw, availH / bh))
  const cx = (minX + maxX) / 2
  const cy = (minY + maxY) / 2
  return { scale, x: width / 2 - cx * scale, y: height / 2 - cy * scale }
}
