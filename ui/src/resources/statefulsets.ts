/**
 * StatefulSets (007-topology-graph-view T017a): built from the shared workload descriptor,
 * same pattern as `deployments.ts`. Needed so the Topology Graph View's workload-tier nodes
 * (FR-001) open a real inspector like every other kind, not the generic "not ported yet" state.
 */
import { workloadDescriptor } from './workloads'

export const statefulsets = workloadDescriptor({
  section: 'workloads/statefulsets',
  kind: 'StatefulSet',
  label: 'StatefulSets',
  listEvent: 'StatefulSetsUpdated',
  readEvent: 'StatefulSetDetailsUpdated',
  refresh: { type: 'RefreshStatefulSets' },
  select: (namespace, name) => ({ type: 'SelectStatefulSet', data: { namespace, name } }),
  scale: (namespace, name, replicas) => ({
    type: 'ScaleStatefulSet',
    data: { namespace, name, replicas },
  }),
  restart: (namespace, name) => ({ type: 'RestartStatefulSet', data: { namespace, name } }),
  delete: (namespace, name) => ({ type: 'DeleteStatefulSet', data: { namespace, name } }),
})
