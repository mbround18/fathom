/**
 * One list fixture per registered resource kind, keyed by section (T059; T075 extends this to
 * every kind). Each entry is the `ResourceListEvent` the backend emits for that section.
 */
import type { ResourceListEvent } from '../../src/protocol/types'
import { PODS_500 } from './batches'
import { genDeployments } from './gen-pods'

export const KIND_LIST_EVENTS: Record<string, ResourceListEvent> = {
  'workloads/pods': { type: 'PodsUpdated', data: PODS_500 },
  'workloads/deployments': { type: 'DeploymentsUpdated', data: genDeployments('kind-a') },
}
