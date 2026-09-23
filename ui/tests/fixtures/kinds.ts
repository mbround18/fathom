/**
 * One list fixture per registered resource kind, keyed by section (T059; T075 extends this to
 * every kind). Each entry is the `ResourceListEvent` the backend emits for that section.
 */
import type {
  ApplicationSummary,
  HelmReleaseSummary,
  NodeSummary,
  ResourceListEvent,
  ServiceSummary,
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

const SERVICES: ServiceSummary[] = [
  {
    name: 'web',
    namespace: 'default',
    service_type: 'ClusterIP',
    cluster_ip: '10.0.0.1',
    external_ips: [],
    port_count: 1,
  },
  {
    name: 'web-lb',
    namespace: 'default',
    service_type: 'LoadBalancer',
    cluster_ip: '10.0.0.2',
    external_ips: ['203.0.113.1'],
    port_count: 2,
  },
]

export const KIND_LIST_EVENTS: Record<string, ResourceListEvent> = {
  'workloads/pods': { type: 'PodsUpdated', data: PODS_500 },
  'workloads/deployments': { type: 'DeploymentsUpdated', data: genDeployments('kind-a') },
  nodes: { type: 'NodesUpdated', data: NODES },
  applications: { type: 'ApplicationsUpdated', data: APPLICATIONS },
  'helm/releases': { type: 'HelmReleasesUpdated', data: HELM_RELEASES },
  'network/services': { type: 'ServicesUpdated', data: SERVICES },
}
