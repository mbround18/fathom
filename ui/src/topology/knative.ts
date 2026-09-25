/**
 * Knative traffic-split graph (028-knative-pane): one Route node fanning out to one node per
 * Revision it targets, each edge labelled with its traffic percentage. Built from
 * `store.knative.overview` (not the cluster-topology `lists`/`details` path) with fixed positions
 * so the small graph is deterministic and needs no force simulation.
 */
import type { KnativeOverview } from '@/protocol/types'

import type { TopologyEdge, TopologyGraph, TopologyNode } from './types'

const KNATIVE_SECTION = 'integrations/knative'

export function buildKnativeTrafficGraph(
  overview: KnativeOverview,
  routeKey: { namespace: string; name: string },
): TopologyGraph {
  const route = overview.routes.find(
    (r) => r.namespace === routeKey.namespace && r.name === routeKey.name,
  )
  if (!route) return { nodes: [], edges: [] }

  const routeId = `route/${route.namespace}/${route.name}`
  const n = route.traffic.length
  const nodes: TopologyNode[] = [
    {
      id: routeId,
      ref: { section: KNATIVE_SECTION, kind: 'Route', namespace: route.namespace, name: route.name },
      kind: 'KnativeRoute',
      label: route.name,
      tone: route.ready === 'Ready' ? 'ok' : route.ready === 'NotReady' ? 'err' : 'muted',
      tier: 'workload',
      x: 0,
      y: 0,
    },
  ]
  const edges: TopologyEdge[] = []
  route.traffic.forEach((t, i) => {
    const id = `revision/${route.namespace}/${t.revision_name}`
    if (!nodes.some((x) => x.id === id)) {
      nodes.push({
        id,
        ref: {
          section: KNATIVE_SECTION,
          kind: 'Revision',
          namespace: route.namespace,
          name: t.revision_name,
        },
        kind: 'KnativeRevision',
        label: t.tag ? `${t.revision_name} (${t.tag})` : t.revision_name,
        tone: t.revision_missing ? 'err' : 'ok',
        detail: t.revision_missing ? 'Revision not found' : undefined,
        tier: 'workload',
        x: 220,
        y: (i - (n - 1) / 2) * 70,
      })
    }
    edges.push({
      id: `${routeId}->${id}#${String(i)}`,
      source: routeId,
      target: id,
      relations: ['traffic'],
      label: `${String(t.percent)}%`,
    })
  })
  return { nodes, edges }
}
