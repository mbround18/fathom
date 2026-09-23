/**
 * Topology Graph View (007-topology-graph-view) data types. See
 * specs/007-topology-graph-view/data-model.md.
 *
 * v1 node kinds are the subset of FR-001's list that already has a real Inspector in this app
 * (research.md R7 / data-model.md): Namespace, Application, HelmRelease, Deployment, StatefulSet,
 * DaemonSet, Pod, Node. `Service` and cluster-scoped `ReplicaSet` browsing are not yet ported to
 * the frontend descriptor registry (no list/inspector exists to click into for them at all, same
 * pre-existing gap `related.rs` documents for `ServiceAccount`/CRDs) and are therefore not
 * rendered as graph nodes in v1 — a known, documented scope limit, not a rendering error.
 */
import type { Tone } from '@/resources/search'
import type { ObjectRef } from '@/resources/types'

export type TopologyTier = 'namespace' | 'workload' | 'pod'

export type TopologyNodeKind =
  | 'Namespace'
  | 'Application'
  | 'HelmRelease'
  | 'Deployment'
  | 'StatefulSet'
  | 'DaemonSet'
  | 'Pod'
  | 'Node'

export interface TopologyNode {
  id: string
  ref: ObjectRef
  kind: TopologyNodeKind
  label: string
  tone: Tone
  detail?: string
  tier: TopologyTier
  parentId?: string
  x?: number
  y?: number
  vx?: number
  vy?: number
  loading?: boolean
}

export interface TopologyEdge {
  id: string
  source: string
  target: string
  relations: string[]
}

export interface TopologyGraph {
  nodes: TopologyNode[]
  edges: TopologyEdge[]
}

/** Per-pane viewport, persisted subset lives on `PaneState.topology` (research.md R4). */
export interface TopologyViewport {
  scale: number
  x: number
  y: number
  namespaceFocus: ReadonlySet<string>
  workloadFocus: ReadonlySet<string>
}

export function nodeId(kind: string, namespace: string | null, name: string): string {
  return `${kind}|${namespace ?? ''}|${name}`
}

export function workloadFocusKey(kind: string, namespace: string, name: string): string {
  return `${namespace}/${kind}/${name}`
}
