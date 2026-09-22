/**
 * One list fixture per registered resource kind, keyed by section (T059; T075 extends this to
 * every kind). Each entry is the `ResourceListEvent` the backend emits for that section.
 */
import type { NodeSummary, ResourceListEvent } from '../../src/protocol/types'
import { PODS_500 } from './batches'
import { genDeployments } from './gen-pods'

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

export const KIND_LIST_EVENTS: Record<string, ResourceListEvent> = {
  'workloads/pods': { type: 'PodsUpdated', data: PODS_500 },
  'workloads/deployments': { type: 'DeploymentsUpdated', data: genDeployments('kind-a') },
  nodes: { type: 'NodesUpdated', data: NODES },
}
