/**
 * DaemonSets (007-topology-graph-view T017a): built from the shared workload descriptor, same
 * pattern as `deployments.ts`. No `scale` action — DaemonSets aren't horizontally scalable
 * (protocol v1 has no `ScaleDaemonSet` command).
 */
import { workloadDescriptor } from './workloads'

export const daemonsets = workloadDescriptor({
  section: 'workloads/daemonsets',
  kind: 'DaemonSet',
  label: 'DaemonSets',
  listEvent: 'DaemonSetsUpdated',
  readEvent: 'DaemonSetDetailsUpdated',
  refresh: { type: 'RefreshDaemonSets' },
  select: (namespace, name) => ({ type: 'SelectDaemonSet', data: { namespace, name } }),
  restart: (namespace, name) => ({ type: 'RestartDaemonSet', data: { namespace, name } }),
  delete: (namespace, name) => ({ type: 'DeleteDaemonSet', data: { namespace, name } }),
})
