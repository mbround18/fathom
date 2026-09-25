/** ReplicaSets (batch A), from the shared workload descriptor: scale, restart, delete. */
import { workloadDescriptor } from './workloads'

export const replicasets = workloadDescriptor({
  section: 'workloads/replicasets',
  kind: 'ReplicaSet',
  label: 'ReplicaSets',
  listEvent: 'ReplicaSetsUpdated',
  readEvent: 'ReplicaSetDetailsUpdated',
  refresh: { type: 'RefreshReplicaSets' },
  select: (namespace, name) => ({ type: 'SelectReplicaSet', data: { namespace, name } }),
  scale: (namespace, name, replicas) => ({
    type: 'ScaleReplicaSet',
    data: { namespace, name, replicas },
  }),
  restart: (namespace, name) => ({ type: 'RestartReplicaSet', data: { namespace, name } }),
  delete: (namespace, name) => ({ type: 'DeleteReplicaSet', data: { namespace, name } }),
})
