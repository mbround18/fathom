/**
 * Pure graph builder (contracts/topology-graph-module.md, data-model.md). Never throws, never
 * dispatches — callers (TopologyCanvas) are responsible for issuing fetches when a node comes
 * back `loading: true`.
 */
import type {
  ApplicationSummary,
  HelmReleaseSummary,
  NodeSummary,
  RelatedResourceRef,
  WorkloadSummary,
} from '@/protocol/types'
// `@/store` must be imported before any descriptor module that (transitively, via a workload
// descriptor's `@/store` import for pod drill-through) imports `@/store` itself — otherwise the
// reentrant cycle leaves that descriptor's module mid-evaluation (see tests/unit/actions.test.ts).
import '@/store'

import { applications } from '@/resources/applications'
import { daemonsets } from '@/resources/daemonsets'
import { deployments } from '@/resources/deployments'
import { helm } from '@/resources/helm'
import { nodes as nodesDescriptor } from '@/resources/nodes'
import { statefulsets } from '@/resources/statefulsets'
import type { ObjectRef } from '@/resources/types'

import { NEUTRAL_TONE, nodeSummaryTone, podTone, workloadTone } from './status'
import type { TopologyEdge, TopologyGraph, TopologyNode, TopologyViewport } from './types'
import { nodeId, workloadFocusKey } from './types'

export interface ListSnapshot {
  status: 'idle' | 'loading' | 'ready' | 'error'
  rows: readonly unknown[]
}

export interface DetailSnapshot {
  eventType: string
  data: unknown
}

/** The slice of `StoreState` this module needs, decoupled for unit testing (no Zustand). */
export interface BuildInput {
  namespaces: readonly string[]
  /** Keyed exactly like `s.lists`, by `listKey(...)`. */
  lists: Record<string, ListSnapshot | undefined>
  /** Keyed exactly like `s.details`, by `detailKey(...)`. */
  details: Record<string, DetailSnapshot | undefined>
  viewport: Pick<TopologyViewport, 'namespaceFocus' | 'workloadFocus'>
  listKeyFor: (section: string) => string
  detailKeyFor: (kind: string, namespace: string | null, name: string) => string
}

const WORKLOAD_KINDS = [
  { kind: 'Deployment' as const, descriptor: deployments },
  { kind: 'StatefulSet' as const, descriptor: statefulsets },
  { kind: 'DaemonSet' as const, descriptor: daemonsets },
]

function ref(section: string, kind: string, namespace: string | null, name: string): ObjectRef {
  return { section, kind, namespace, name }
}

function edgeId(a: string, b: string): string {
  return [a, b].sort().join('|')
}

class EdgeBuilder {
  private readonly byId = new Map<string, TopologyEdge>()

  add(a: string, b: string, relation: string): void {
    if (a === b) return
    const id = edgeId(a, b)
    const existing = this.byId.get(id)
    if (existing) {
      if (!existing.relations.includes(relation)) existing.relations.push(relation)
      return
    }
    this.byId.set(id, { id, source: a, target: b, relations: [relation] })
  }

  list(): TopologyEdge[] {
    return [...this.byId.values()]
  }
}

export function buildTopologyGraph(input: BuildInput): TopologyGraph {
  const { namespaces, lists, details, viewport, listKeyFor, detailKeyFor } = input
  const nodes: TopologyNode[] = []
  const edges = new EdgeBuilder()
  const nodesByKind: Record<string, TopologyNode> = {}

  const put = (n: TopologyNode): TopologyNode => {
    nodes.push(n)
    nodesByKind[n.id] = n
    return n
  }

  // 1. Namespace tier: always present (FR-004), from the session's namespace list (no fetch).
  for (const ns of namespaces) {
    put({
      id: nodeId('Namespace', null, ns),
      ref: ref('namespaces', 'Namespace', null, ns),
      kind: 'Namespace',
      label: ns,
      tone: NEUTRAL_TONE,
      tier: 'namespace',
    })
  }

  const appRows = (lists[listKeyFor(applications.section)]?.rows ?? []) as ApplicationSummary[]
  const helmRows = (lists[listKeyFor(helm.section)]?.rows ?? []) as HelmReleaseSummary[]
  const nodeRows = (lists[listKeyFor(nodesDescriptor.section)]?.rows ?? []) as NodeSummary[]

  // 2. Namespace -> Application/HelmRelease containment (FR-004), no fetch needed (row already
  // carries `namespace`).
  for (const a of appRows) {
    const id = nodeId('Application', a.namespace, a.instance)
    put({
      id,
      ref: ref(applications.section, 'Application', a.namespace, a.instance),
      kind: 'Application',
      label: a.name,
      tone: a.ready_count >= a.total_count && a.total_count > 0 ? 'ok' : 'warn',
      detail: `${String(a.ready_count)}/${String(a.total_count)}`,
      tier: 'namespace',
    })
    if (namespaces.includes(a.namespace)) {
      edges.add(nodeId('Namespace', null, a.namespace), id, 'owns')
    }
  }
  for (const h of helmRows) {
    const id = nodeId('HelmRelease', h.namespace, h.name)
    put({
      id,
      ref: ref(helm.section, 'HelmRelease', h.namespace, h.name),
      kind: 'HelmRelease',
      label: h.name,
      tone: h.degraded ? 'err' : h.status === 'Deployed' ? 'ok' : 'warn',
      detail: h.status,
      tier: 'namespace',
    })
    if (namespaces.includes(h.namespace)) {
      edges.add(nodeId('Namespace', null, h.namespace), id, 'owns')
    }
  }

  // Node (kube Node) tier: small, always rendered (data-model.md step 6).
  for (const n of nodeRows) {
    put({
      id: nodeId('Node', null, n.name),
      ref: ref(nodesDescriptor.section, 'Node', null, n.name),
      kind: 'Node',
      label: n.name,
      tone: nodeSummaryTone(n),
      detail: `${String(n.pod_count)}/${String(n.pod_capacity)} pods`,
      tier: 'namespace',
    })
  }

  // 3. Workload tier: only for namespaces the operator has zoomed into.
  for (const ns of viewport.namespaceFocus) {
    for (const { kind, descriptor } of WORKLOAD_KINDS) {
      const rows = (lists[listKeyFor(descriptor.section)]?.rows ?? []) as WorkloadSummary[]
      for (const w of rows.filter((r) => r.namespace === ns)) {
        const id = nodeId(kind, w.namespace, w.name)
        put({
          id,
          ref: ref(descriptor.section, kind, w.namespace, w.name),
          kind,
          label: w.name,
          tone: workloadTone(w),
          detail: `${String(w.ready)}/${String(w.desired)} ready`,
          tier: 'workload',
          parentId: nodeId('Namespace', null, ns),
        })
        edges.add(nodeId('Namespace', null, ns), id, 'owns')

        // Application -> owns -> workload, from that Application's already-fetched
        // RelatedResourceRef[] (research.md R2: no fetch issued here; build.ts stays pure).
        for (const a of appRows.filter((row) => row.namespace === ns)) {
          const related = relatedResources(details, detailKeyFor, a.namespace, a.instance)
          if (related?.some((r) => r.kind === kind && r.name === w.name)) {
            edges.add(nodeId('Application', a.namespace, a.instance), id, 'owns')
          }
        }
      }

      // 4. Pod tier: only for workloads the operator has zoomed into further.
      for (const w of rows.filter((r) => r.namespace === ns)) {
        const focusKey = workloadFocusKey(kind, w.namespace, w.name)
        if (!viewport.workloadFocus.has(focusKey)) continue
        const workloadId = nodeId(kind, w.namespace, w.name)
        const detail = details[detailKeyFor(kind, w.namespace, w.name)]
        if (!detail || detail.eventType !== `${kind}DetailsUpdated`) {
          const parent = nodesByKind[workloadId]
          if (parent) parent.loading = true
          continue
        }
        const pods = (detail.data as { pods?: { name: string; namespace: string; phase: string; ready: number; total: number; node: string }[] }).pods ?? []
        for (const p of pods) {
          const podNodeId = nodeId('Pod', p.namespace, p.name)
          put({
            id: podNodeId,
            ref: ref('workloads/pods', 'Pod', p.namespace, p.name),
            kind: 'Pod',
            label: p.name,
            tone: podTone(p),
            detail: `${p.phase} ${String(p.ready)}/${String(p.total)}`,
            tier: 'pod',
            parentId: workloadId,
          })
          edges.add(workloadId, podNodeId, 'owns')
          const nodeNodeId = nodeId('Node', null, p.node)
          if (p.node && nodesByKind[nodeNodeId]) {
            edges.add(nodeNodeId, podNodeId, 'scheduled-on')
          }
        }
      }
    }
  }

  return { nodes, edges: edges.list() }
}

function relatedResources(
  details: BuildInput['details'],
  detailKeyFor: BuildInput['detailKeyFor'],
  namespace: string | null,
  name: string,
): RelatedResourceRef[] | undefined {
  const state = details[detailKeyFor('RelatedResources', namespace, name)]
  if (!state || state.eventType !== 'RelatedResourcesUpdated') return undefined
  return (state.data as { related: RelatedResourceRef[] }).related
}
