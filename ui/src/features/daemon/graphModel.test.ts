import { describe, expect, it } from 'vitest'

import type { TopologyEdge, TopologyNode } from '@/protocol/types'

import {
  buildModel,
  capNodes,
  diffModels,
  emptyModel,
  layoutLanes,
  NODE_CAP,
  shapeFor,
} from './graphModel'

const n = (uid: string, kind = 'Pod', health: TopologyNode['health'] = 'ok'): TopologyNode => ({
  uid,
  kind,
  namespace: 'ns',
  name: uid,
  health,
})
const rec = (list: TopologyNode[]) => Object.fromEntries(list.map((x) => [x.uid, x]))
const edge = (from: string, to: string): TopologyEdge => ({ from, to, relation: 'owns' })

describe('graphModel', () => {
  it('maps kind to shape and health to ring', () => {
    const m = buildModel(rec([n('a', 'Deployment', 'error'), n('b', 'Weird')]), [], null)
    expect(m.nodes.get('a')).toMatchObject({ shape: shapeFor('Deployment'), health: 'unhealthy' })
    expect(m.nodes.get('b')?.shape).toBe('ellipse')
  })

  it('drops edges whose endpoints are not visible', () => {
    const m = buildModel(rec([n('a')]), [edge('a', 'gone')], null)
    expect(m.edges.size).toBe(0)
  })

  it('lays kinds out in ordered lanes deterministically', () => {
    const pos = layoutLanes([n('p'), n('d', 'Deployment'), n('x', 'Node')])
    expect(pos.get('x')?.x).toBeLessThan(pos.get('d')?.x ?? 0)
    expect(pos.get('d')?.x).toBeLessThan(pos.get('p')?.x ?? 0)
  })

  it('caps by health severity and always keeps the selection', () => {
    const list = [
      ...Array.from({ length: NODE_CAP + 50 }, (_, i) => n(`ok${i}`)),
      n('bad', 'Pod', 'error'),
    ]
    const { visible, hidden } = capNodes(list, 'ok1549')
    expect(visible).toHaveLength(NODE_CAP)
    expect(hidden).toBe(51)
    expect(visible.some((v) => v.uid === 'bad')).toBe(true)
    expect(visible.some((v) => v.uid === 'ok1549')).toBe(true)
  })

  it('drops labels at high node counts', () => {
    const big = rec(Array.from({ length: 400 }, (_, i) => n(`p${i}`)))
    expect(buildModel(big, [], null).labels).toBe(false)
  })

  it('diffs incrementally without moving existing nodes or relayout', () => {
    const base = rec(Array.from({ length: 20 }, (_, i) => n(`p${i}`)))
    const prev = buildModel(base, [edge('p0', 'p1')], null)
    const next = buildModel(
      { ...base, p2: n('p2', 'Pod', 'warning'), extra: n('extra') },
      [edge('p0', 'p1'), edge('p0', 'extra')],
      null,
    )
    const d = diffModels(prev, next)
    expect(d.addNodes.map((x) => x.id)).toEqual(['extra'])
    expect(d.updateNodes.map((x) => x.id)).toEqual(['p2'])
    expect(d.addEdges).toHaveLength(1)
    expect(d.removeNodes).toEqual([])
    expect(d.relayout).toBe(false)
  })

  it('requests a relayout on first load and removal', () => {
    const m = buildModel(rec([n('a')]), [], null)
    expect(diffModels(emptyModel(), m).relayout).toBe(true)
    const d = diffModels(m, buildModel({}, [], null))
    expect(d.removeNodes).toEqual(['a'])
  })
})
