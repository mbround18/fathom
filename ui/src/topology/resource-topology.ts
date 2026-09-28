/**
 * 043 per-resource topology (FR-031): split the related-resources resolver's neighbours
 * (`RelatedResourcesUpdated`, spec 004) into upstream and downstream for the inspector's
 * Topology tab. Pure; the tab owns the request and the superseded-root check.
 *
 * A ref's `relation` describes the root's link to the neighbour (`src/backend/related.rs`,
 * `helm.rs`): the root is owned by, a member of, scaled by, mounted by or reconciled by an
 * upstream neighbour. Everything else, including relations this build does not know, is
 * downstream and keeps its relation name as the label.
 */
import type { RelatedResourceRef } from '@/protocol/types'

export const UPSTREAM: ReadonlySet<string> = new Set([
  'owned-by',
  'member-of',
  'scaledBy',
  'mounted-by',
  'reconciled-by',
])

/** Kinds the related-resources resolver covers; the Topology tab is shown only for these. */
export const RESOLVER_KINDS: ReadonlySet<string> = new Set([
  'Node',
  'Service',
  'PersistentVolumeClaim',
  'Pod',
  'Deployment',
  'StatefulSet',
  'DaemonSet',
  'ReplicaSet',
  'ScaledObject',
  'ScaledJob',
  'Application',
  'HelmRelease',
])

export interface TopologyRoot {
  kind: string
  namespace: string | null
  name: string
}

export type Neighbour = RelatedResourceRef & { relationLabel: string }

export interface ResourceTopology {
  root: TopologyRoot
  upstream: Neighbour[]
  downstream: Neighbour[]
}

const LABELS: Readonly<Record<string, string>> = {
  'owned-by': 'owned by',
  'member-of': 'member of',
  scaledBy: 'scaled by',
  'mounted-by': 'mounted by',
  'reconciled-by': 'reconciled by',
  owns: 'owns',
  selects: 'selects',
  scales: 'scales',
  'scheduled-on': 'scheduled on',
}

/** Readable label for a relation; an unknown relation keeps its own name. */
export function relationLabel(relation: string): string {
  return LABELS[relation] ?? relation
}

export function splitRelated(
  root: TopologyRoot,
  refs: readonly RelatedResourceRef[],
): ResourceTopology {
  const upstream: Neighbour[] = []
  const downstream: Neighbour[] = []
  for (const ref of refs) {
    const n: Neighbour = { ...ref, relationLabel: relationLabel(ref.relation) }
    ;(UPSTREAM.has(ref.relation) ? upstream : downstream).push(n)
  }
  return { root, upstream, downstream }
}
