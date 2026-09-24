/**
 * Every static fixture batch, by name (the contract test walks this). Scenario-driven replies
 * are built from the same builders in ./scenarios.ts.
 */
import type { EventBatch } from '../../src/protocol/types'
import {
  backendStarted,
  clusterVersionHealthy,
  clusterVersionSkewed,
  clusterVersionUnknown,
  contextChanged,
  deployments,
  deploymentsKindA,
  droppedWarning,
  emptyContexts,
  federationContexts,
  federationStatuses,
  namespaces,
  noKubeconfig,
  podDetailKindA,
  podEventsKindA,
  pods,
  pods10k,
  pods500,
  podsError,
  requestNewer,
  requestOlder,
  staleKindA,
  twoContexts,
  uiConfigDefault,
  uiConfigSaved,
} from './batches'

export const FIXTURES = {
  'backend-started': backendStarted,
  'ui-config-default': uiConfigDefault,
  'ui-config-saved': uiConfigSaved,
  'two-contexts': twoContexts,
  'empty-contexts': emptyContexts,
  'no-kubeconfig': noKubeconfig,
  'context-changed-kind-b': contextChanged('kind-b'),
  'cluster-version-healthy': clusterVersionHealthy,
  'cluster-version-skewed': clusterVersionSkewed,
  'cluster-version-unknown': clusterVersionUnknown,
  'federation-contexts': federationContexts,
  'federation-statuses': federationStatuses,
  'namespaces-kind-a': namespaces('kind-a'),
  'namespaces-kind-b': namespaces('kind-b'),
  'pods-500': pods500,
  'pods-kind-b': pods('kind-b'),
  'pods-10k': pods10k(),
  deployments: deploymentsKindA,
  'deployments-kind-b': deployments('kind-b'),
  'pod-detail': podDetailKindA,
  'pod-events': podEventsKindA,
  'stale-kind-a': staleKindA,
  'request-newer': requestNewer,
  'request-older': requestOlder,
  'pods-error': podsError,
  'dropped-warning': droppedWarning,
} satisfies Record<string, EventBatch>

export type FixtureName = keyof typeof FIXTURES

export * from './batches'
export { SCENARIOS, isScenarioName, type Scenario, type ScenarioName } from './scenarios'
