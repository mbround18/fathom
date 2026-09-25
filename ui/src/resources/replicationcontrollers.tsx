/** ReplicationControllers (batch A), from the shared workload descriptor: scale, restart, delete. */
import { workloadDescriptor } from './workloads'

export const replicationcontrollers = workloadDescriptor({
  section: 'workloads/replicationcontrollers',
  kind: 'ReplicationController',
  label: 'ReplicationControllers',
  listEvent: 'ReplicationControllersUpdated',
  readEvent: 'ReplicationControllerDetailsUpdated',
  refresh: { type: 'RefreshReplicationControllers' },
  select: (namespace, name) => ({
    type: 'SelectReplicationController',
    data: { namespace, name },
  }),
  scale: (namespace, name, replicas) => ({
    type: 'ScaleReplicationController',
    data: { namespace, name, replicas },
  }),
  restart: (namespace, name) => ({
    type: 'RestartReplicationController',
    data: { namespace, name },
  }),
  delete: (namespace, name) => ({
    type: 'DeleteReplicationController',
    data: { namespace, name },
  }),
})
