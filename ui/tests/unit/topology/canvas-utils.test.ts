import { describe, expect, it } from 'vitest'

import {
  clampScale,
  FIT_MAX_SCALE,
  fitCamera,
  hitTest,
  tokenColor,
  screenToWorld,
  worldToScreen,
  zoomAt,
} from '@/components/topology/TopologyCanvas.utils'
import type { TopologyNode } from '@/topology/types'

describe('screenToWorld / worldToScreen', () => {
  it('round-trips through a non-trivial camera', () => {
    const camera = { scale: 2, x: 50, y: -20 }
    const world = screenToWorld(camera, 130, 80)
    const back = worldToScreen(camera, world.x, world.y)
    expect(back.x).toBeCloseTo(130)
    expect(back.y).toBeCloseTo(80)
  })
})

describe('zoomAt', () => {
  it('keeps the world point under the cursor fixed on screen', () => {
    const camera = { scale: 1, x: 0, y: 0 }
    const before = screenToWorld(camera, 100, 100)
    const after = zoomAt(camera, 2, 100, 100)
    const afterWorld = screenToWorld(after, 100, 100)
    expect(afterWorld.x).toBeCloseTo(before.x)
    expect(afterWorld.y).toBeCloseTo(before.y)
    expect(after.scale).toBeCloseTo(2)
  })

  it('clamps to the min/max scale', () => {
    expect(zoomAt({ scale: 0.2, x: 0, y: 0 }, 0.01, 0, 0).scale).toBe(clampScale(0.2 * 0.01))
    expect(zoomAt({ scale: 5, x: 0, y: 0 }, 100, 0, 0).scale).toBe(clampScale(5 * 100))
  })
})

describe('hitTest', () => {
  const nodes: TopologyNode[] = [
    {
      id: 'a',
      ref: { section: 'namespaces', kind: 'Namespace', namespace: null, name: 'a' },
      kind: 'Namespace',
      label: 'a',
      tone: 'ok',
      tier: 'namespace',
      x: 0,
      y: 0,
    },
    {
      id: 'b',
      ref: { section: 'namespaces', kind: 'Namespace', namespace: null, name: 'b' },
      kind: 'Namespace',
      label: 'b',
      tone: 'ok',
      tier: 'namespace',
      x: 200,
      y: 200,
    },
  ]

  it('finds the node under a point within its radius', () => {
    expect(hitTest({ x: 2, y: -1 }, nodes)?.id).toBe('a')
  })

  it('returns undefined when nothing is under the point', () => {
    expect(hitTest({ x: 1000, y: 1000 }, nodes)).toBeUndefined()
  })

  it('skips nodes with no position yet', () => {
    const base = nodes[0]
    if (!base) throw new Error('fixture missing')
    const withUnpositioned: TopologyNode[] = [{ ...base, x: undefined, y: undefined }]
    expect(hitTest({ x: 0, y: 0 }, withUnpositioned)).toBeUndefined()
  })
})

describe('tokenColor', () => {
  it('passes complete colors through and wraps bare oklch channels', () => {
    expect(tokenColor(' oklch(51% 0.11 163) ', '#888')).toBe('oklch(51% 0.11 163)')
    expect(tokenColor('#39ffb0', '#888')).toBe('#39ffb0')
    expect(tokenColor('51% 0.11 163', '#888')).toBe('oklch(51% 0.11 163)')
    expect(tokenColor('', '#888')).toBe('#888')
  })
})

describe('fitCamera', () => {
  const node = (id: string, x: number, y: number): TopologyNode => ({
    id,
    ref: { section: 'namespaces', kind: 'Namespace', namespace: null, name: id },
    kind: 'Namespace',
    label: id,
    tone: 'muted',
    tier: 'namespace',
    x,
    y,
  })

  it('puts every node inside the viewport', () => {
    const nodes = [node('a', -900, -400), node('b', 1200, 700), node('c', 0, 0)]
    const cam = fitCamera(nodes, 320, 600)
    for (const n of nodes) {
      const p = worldToScreen(cam, n.x ?? 0, n.y ?? 0)
      expect(p.x).toBeGreaterThanOrEqual(0)
      expect(p.x).toBeLessThanOrEqual(320)
      expect(p.y).toBeGreaterThanOrEqual(0)
      expect(p.y).toBeLessThanOrEqual(600)
    }
  })

  it('never zooms past the cap for a sparse graph', () => {
    expect(fitCamera([node('a', 10, 10)], 1600, 900).scale).toBeLessThanOrEqual(FIT_MAX_SCALE)
  })

  it('falls back to identity with no positioned nodes', () => {
    expect(fitCamera([], 100, 100)).toEqual({ scale: 1, x: 0, y: 0 })
  })
})
