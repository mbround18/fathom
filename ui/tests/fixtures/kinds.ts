/**
 * One list fixture per registered resource kind, keyed by section (T059; T075 extends this to
 * every kind). Each entry is the `ResourceListEvent` the backend emits for that section.
 */
import type {
  ApplicationSummary,
  HelmReleaseSummary,
  NodeSummary,
  ResourceListEvent,
} from '../../src/protocol/types'
import { PODS_500 } from './batches'
import { genDeployments } from './gen-pods'

const APPLICATIONS: ApplicationSummary[] = [
  {
    instance: 'redis',
    name: 'redis',
    namespace: 'default',
    managed_by: 'Helm',
    version: '7.2.4',
    creation_timestamp: '2024-01-15T10:00:00Z',
    ready_count: 1,
    total_count: 1,
  },
]

const NODES: NodeSummary[] = [
  {
    name: 'kind-a-control-plane',
    status: 'Ready',
    role: 'control-plane',
    pod_count: 12,
    pod_capacity: 110,
    cpu_allocatable_millicores: 4000,
    memory_allocatable_bytes: 8_000_000_000,
  },
  {
    name: 'kind-a-worker',
    status: 'Ready',
    role: 'worker',
    pod_count: 8,
    pod_capacity: 110,
    cpu_allocatable_millicores: 2000,
    memory_allocatable_bytes: 4_000_000_000,
  },
]

const HELM_RELEASES: HelmReleaseSummary[] = [
  {
    name: 'redis',
    namespace: 'cache',
    chart: 'redis',
    chart_version: '18.6.1',
    app_version: '7.2.4',
    revision: 3,
    status: 'Deployed',
    updated: '2024-01-15T10:00:00Z',
    degraded: false,
  },
]

export const KIND_LIST_EVENTS: Record<string, ResourceListEvent> = {
  'workloads/pods': { type: 'PodsUpdated', data: PODS_500 },
  'workloads/deployments': { type: 'DeploymentsUpdated', data: genDeployments('kind-a') },
  'workloads/statefulsets': { type: 'StatefulSetsUpdated', data: genDeployments('kind-a') },
  'workloads/daemonsets': { type: 'DaemonSetsUpdated', data: genDeployments('kind-a') },
  nodes: { type: 'NodesUpdated', data: NODES },
  applications: { type: 'ApplicationsUpdated', data: APPLICATIONS },
  'helm/releases': { type: 'HelmReleasesUpdated', data: HELM_RELEASES },
}
