/**
 * Automatic layout (FR-009/FR-011): a thin `d3-force` wrapper. Headless — no DOM/SVG coupling —
 * so it composes with the hand-rolled canvas renderer (research.md R1). Runs a bounded number of
 * ticks synchronously and returns positions; the caller decides when to call it (Principle V: no
 * continuous simulation loop once the graph has converged/is idle).
 */
import {
  forceCenter,
  forceCollide,
  forceLink,
  forceManyBody,
  forceSimulation,
  type SimulationNodeDatum,
} from 'd3-force'

import type { TopologyGraph, TopologyNode } from './types'

const MAX_TICKS = 200
export const NODE_RADIUS: Record<TopologyNode['tier'], number> = {
  namespace: 22,
  workload: 14,
  pod: 6,
}

type SimNode = TopologyNode & SimulationNodeDatum

export function layoutTopologyGraph(
  graph: TopologyGraph,
  prior?: ReadonlyMap<string, { x: number; y: number }>,
  size: { width: number; height: number } = { width: 800, height: 600 },
): TopologyGraph {
  if (graph.nodes.length === 0) return graph

  const simNodes: SimNode[] = graph.nodes.map((n) => {
    const seed = prior?.get(n.id)
    return {
      ...n,
      x: seed?.x ?? n.x ?? Math.random() * size.width,
      y: seed?.y ?? n.y ?? Math.random() * size.height,
    }
  })
  const byId = new Map(simNodes.map((n) => [n.id, n]))
  const simLinks = graph.edges
    .filter((e) => byId.has(e.source) && byId.has(e.target))
    .map((e) => ({ source: e.source, target: e.target }))

  const sim = forceSimulation(simNodes)
    .force(
      'link',
      forceLink(simLinks)
        .id((d) => (d as SimNode).id)
        .distance(60)
        .strength(0.4),
    )
    .force('charge', forceManyBody().strength(-120))
    .force('center', forceCenter(size.width / 2, size.height / 2))
    .force(
      'collide',
      forceCollide((d) => NODE_RADIUS[d.tier] + 4),
    )
    .stop()

  for (let i = 0; i < MAX_TICKS; i++) sim.tick()

  const nodes: TopologyNode[] = simNodes.map((n) => ({
    ...n,
    x: n.x ?? 0,
    y: n.y ?? 0,
  }))
  return { nodes, edges: graph.edges }
}

export function snapshotPositions(graph: TopologyGraph): Map<string, { x: number; y: number }> {
  const out = new Map<string, { x: number; y: number }>()
  for (const n of graph.nodes) {
    if (n.x !== undefined && n.y !== undefined) out.set(n.id, { x: n.x, y: n.y })
  }
  return out
}
