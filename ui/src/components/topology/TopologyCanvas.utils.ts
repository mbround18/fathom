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

export const MIN_SCALE = 0.15
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
