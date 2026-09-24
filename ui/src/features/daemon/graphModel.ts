/**
 * 036 cytoscape: pure data mapping for the daemon topology graph (no cytoscape import, so it is
 * unit-testable headless). Nodes get a kind lane (deterministic O(n) layout: no force relayout on
 * every delta), edges a relation style, and huge graphs are capped by level of detail.
 */
import type { TopologyEdge, TopologyHealth, TopologyNode } from '@/protocol/types'

/** Above this many nodes the graph renders a health-prioritised subset (the list stays complete). */
export const NODE_CAP = 1500
/** Above this many visible nodes labels are dropped (level of detail). */
export const LABEL_LIMIT = 250

export const KIND_ORDER = [
  'Node',
  'Namespace',
  'Deployment',
  'StatefulSet',
  'DaemonSet',
  'ReplicaSet',
  'HorizontalPodAutoscaler',
  'Service',
  'Pod',
] as const

export const KIND_SHAPE: Record<string, string> = {
  Node: 'round-rectangle',
  Namespace: 'round-tag',
  Deployment: 'round-hexagon',
  StatefulSet: 'round-hexagon',
  DaemonSet: 'round-hexagon',
  ReplicaSet: 'round-octagon',
  HorizontalPodAutoscaler: 'round-diamond',
  Service: 'round-pentagon',
  Pod: 'ellipse',
}

export const shapeFor = (kind: string): string => KIND_SHAPE[kind] ?? 'ellipse'

export const HEALTH_RING: Record<TopologyHealth, 'healthy' | 'degraded' | 'unhealthy' | 'unknown'> =
  { ok: 'healthy', warning: 'degraded', error: 'unhealthy', unknown: 'unknown' }

export interface EdgeStyle {
  lineStyle: 'solid' | 'dashed' | 'dotted'
  arrow: string
  width: number
}

export const RELATION_STYLE: Record<string, EdgeStyle> = {
  owns: { lineStyle: 'solid', arrow: 'triangle', width: 2 },
  selects: { lineStyle: 'dashed', arrow: 'vee', width: 1.5 },
  scheduledOn: { lineStyle: 'dotted', arrow: 'circle', width: 1.5 },
  scales: { lineStyle: 'dashed', arrow: 'diamond', width: 2 },
}

const laneIndex = (kind: string): number => {
  const i = (KIND_ORDER as readonly string[]).indexOf(kind)
  return i === -1 ? KIND_ORDER.length : i
}

export interface GraphNode {
  id: string
  label: string
  kind: string
  health: string
  shape: string
  x: number
  y: number
}
export interface GraphEdge {
  id: string
  source: string
  target: string
  relation: string
}

export const edgeId = (e: TopologyEdge): string => `${e.from}>${e.to}>${e.relation}`

const SEV: Record<TopologyHealth, number> = { error: 0, warning: 1, unknown: 2, ok: 3 }

/** Health-prioritised cap so a failing object is never the one dropped. Selected is always kept. */
export function capNodes(
  nodes: readonly TopologyNode[],
  selected: string | null,
  cap = NODE_CAP,
): { visible: TopologyNode[]; hidden: number } {
  if (nodes.length <= cap) return { visible: [...nodes], hidden: 0 }
  const sorted = [...nodes].sort(
    (a, b) => SEV[a.health] - SEV[b.health] || a.uid.localeCompare(b.uid),
  )
  const visible = sorted.slice(0, cap)
  if (selected && !visible.some((n) => n.uid === selected)) {
    const s = nodes.find((n) => n.uid === selected)
    if (s) visible[cap - 1] = s
  }
  return { visible, hidden: nodes.length - visible.length }
}

const COL_W = 190
const ROW_H = 46
const PER_COL = 40

/** Lane layout: one column group per present kind (ordered), rows sorted by name. Deterministic. */
export function layoutLanes(nodes: readonly TopologyNode[]): Map<string, { x: number; y: number }> {
  const lanes = new Map<number, TopologyNode[]>()
  for (const n of nodes) {
    const i = laneIndex(n.kind)
    const l = lanes.get(i) ?? []
    l.push(n)
    lanes.set(i, l)
  }
  const out = new Map<string, { x: number; y: number }>()
  let col = 0
  for (const i of [...lanes.keys()].sort((a, b) => a - b)) {
    const list = (lanes.get(i) ?? []).sort(
      (a, b) => a.name.localeCompare(b.name) || a.uid.localeCompare(b.uid),
    )
    // Tall lanes wrap into extra columns so 1000 pods do not make a 46k px strip.
    list.forEach((n, r) => {
      out.set(n.uid, { x: (col + Math.floor(r / PER_COL)) * COL_W, y: (r % PER_COL) * ROW_H })
    })
    col += Math.max(1, Math.ceil(list.length / PER_COL))
  }
  return out
}

export interface GraphModel {
  nodes: Map<string, GraphNode>
  edges: Map<string, GraphEdge>
  hidden: number
  labels: boolean
}

export const emptyModel = (): GraphModel => ({
  nodes: new Map(),
  edges: new Map(),
  hidden: 0,
  labels: true,
})

export function buildModel(
  nodes: Record<string, TopologyNode>,
  edges: readonly TopologyEdge[],
  selected: string | null,
): GraphModel {
  const { visible, hidden } = capNodes(Object.values(nodes), selected)
  const pos = layoutLanes(visible)
  const map = new Map<string, GraphNode>()
  for (const n of visible) {
    const p = pos.get(n.uid) ?? { x: 0, y: 0 }
    map.set(n.uid, {
      id: n.uid,
      label: n.name,
      kind: n.kind,
      health: HEALTH_RING[n.health],
      shape: shapeFor(n.kind),
      ...p,
    })
  }
  const em = new Map<string, GraphEdge>()
  for (const e of edges) {
    if (map.has(e.from) && map.has(e.to))
      em.set(edgeId(e), { id: edgeId(e), source: e.from, target: e.to, relation: e.relation })
  }
  return { nodes: map, edges: em, hidden, labels: map.size <= LABEL_LIMIT }
}

export interface ModelDiff {
  addNodes: GraphNode[]
  removeNodes: string[]
  /** Existing nodes whose health/label changed (style-only update; never moved). */
  updateNodes: GraphNode[]
  addEdges: GraphEdge[]
  removeEdges: string[]
  /** True when a fit/relayout is warranted (initial load or a large churn). */
  relayout: boolean
}

/**
 * Incremental diff: existing nodes keep their position (no jank). A full lane re-position is only
 * requested when more than a quarter of the graph was added/removed at once.
 */
export function diffModels(prev: GraphModel, next: GraphModel): ModelDiff {
  const addNodes: GraphNode[] = []
  const updateNodes: GraphNode[] = []
  for (const [id, n] of next.nodes) {
    const p = prev.nodes.get(id)
    if (!p) addNodes.push(n)
    else if (p.health !== n.health || p.label !== n.label) updateNodes.push(n)
  }
  const removeNodes = [...prev.nodes.keys()].filter((id) => !next.nodes.has(id))
  const addEdges = [...next.edges.values()].filter((e) => !prev.edges.has(e.id))
  const removeEdges = [...prev.edges.keys()].filter((id) => !next.edges.has(id))
  const churn = addNodes.length + removeNodes.length
  const relayout = prev.nodes.size === 0 || churn > Math.max(8, next.nodes.size / 4)
  return { addNodes, removeNodes, updateNodes, addEdges, removeEdges, relayout }
}
