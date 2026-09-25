import { describe, expect, it } from 'vitest'

import { layoutTopologyGraph, snapshotPositions } from '@/topology/layout'
import type { TopologyGraph } from '@/topology/types'

function graph(): TopologyGraph {
  return {
    nodes: [
      {
        id: 'a',
        ref: { section: 'namespaces', kind: 'Namespace', namespace: null, name: 'a' },
        kind: 'Namespace',
        label: 'a',
        tone: 'muted',
        tier: 'namespace',
      },
      {
        id: 'b',
        ref: { section: 'namespaces', kind: 'Namespace', namespace: null, name: 'b' },
        kind: 'Namespace',
        label: 'b',
        tone: 'muted',
        tier: 'namespace',
      },
    ],
    edges: [{ id: 'a|b', source: 'a', target: 'b', relations: ['owns'] }],
  }
}

describe('layoutTopologyGraph', () => {
  it('assigns finite x/y to every node', () => {
    const out = layoutTopologyGraph(graph())
    for (const n of out.nodes) {
      expect(Number.isFinite(n.x)).toBe(true)
      expect(Number.isFinite(n.y)).toBe(true)
    }
  })

  it('is a no-op on an empty graph', () => {
    const out = layoutTopologyGraph({ nodes: [], edges: [] })
    expect(out.nodes).toHaveLength(0)
  })

  it('seeds from prior positions instead of jumping to random ones', () => {
    const prior = new Map([['a', { x: 500, y: 500 }]])
    const out = layoutTopologyGraph(graph(), prior, { width: 100, height: 100 })
    const a = out.nodes.find((n) => n.id === 'a')
    // Starting near (500,500) with only weak forces pulling toward the small (100,100) center,
    // a bounded number of ticks should not have thrown it back near (0,0).
    expect(a?.x).toBeGreaterThan(50)
  })

  it('snapshotPositions captures every node with a position', () => {
    const out = layoutTopologyGraph(graph())
    const snap = snapshotPositions(out)
    expect(snap.size).toBe(2)
  })
})
