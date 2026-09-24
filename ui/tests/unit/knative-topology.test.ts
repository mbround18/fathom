import { describe, expect, it } from 'vitest'

import type { KnativeOverview, KnativeRouteTrafficTarget } from '@/protocol/types'
import { buildKnativeTrafficGraph } from '@/topology/knative'

import { overview } from './knative-fixtures'

function withTraffic(traffic: KnativeRouteTrafficTarget[]): KnativeOverview {
  const o = overview()
  return { ...o, routes: o.routes.map((r) => ({ ...r, traffic })) }
}

const key = { namespace: 'default', name: 'hello' }

describe('buildKnativeTrafficGraph', () => {
  it('builds an 80/20 split summing to 100', () => {
    const g = buildKnativeTrafficGraph(overview(), key)
    expect(g.edges.map((e) => e.label)).toEqual(['80%', '20%'])
    expect(g.nodes.filter((n) => n.kind === 'KnativeRevision')).toHaveLength(2)
    expect(g.nodes.filter((n) => n.kind === 'KnativeRoute')).toHaveLength(1)
  })

  it('single 100% target yields one edge', () => {
    const o = withTraffic([
      { revision_name: 'hello-00001', percent: 100, tag: null, revision_missing: false },
    ])
    const g = buildKnativeTrafficGraph(o, key)
    expect(g.edges).toHaveLength(1)
    expect(g.edges.map((e) => e.label)).toEqual(['100%'])
  })

  it('keeps a missing revision, tonally distinct', () => {
    const o = withTraffic([
      { revision_name: 'gone', percent: 100, tag: null, revision_missing: true },
    ])
    const g = buildKnativeTrafficGraph(o, key)
    expect(g.edges).toHaveLength(1)
    expect(g.nodes.find((n) => n.kind === 'KnativeRevision')?.tone).toBe('err')
  })

  it('returns an empty graph for an unknown route', () => {
    expect(buildKnativeTrafficGraph(overview(), { namespace: 'x', name: 'y' }).nodes).toEqual([])
  })
})
