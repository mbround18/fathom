/**
 * Recorded-shape `EventBatch` fixtures for US1 (contracts/ipc.md § Verification).
 *
 * Routes and correlations mirror what `src/backend` emits today: System events on
 * `Route::global("system")`, namespaces on `Route::cluster("system", ctx)`, pod/deployment lists
 * on `Route::namespaced(section, ctx, None)`, reads on the object's namespace, and cluster
 * events on `Route::global("events")`. Protocol v1 backends send `Correlation::None`; the
 * `Request` fixtures below exercise the staleness rule for when they don't.
 */
import type { EventBatch, PodSummary, UiConfig } from '../../src/protocol/types'
import { batch, clusterRoute, globalRoute, list, nsRoute, read, SECTIONS, system } from './builders'
import {
  genDeployments,
  genPods,
  NAMESPACES,
  podDetails,
  podEvents,
  type FixtureContext,
} from './gen-pods'

export const CONTEXTS = ['kind-a', 'kind-b'] as const satisfies readonly FixtureContext[]

export const PODS_500 = genPods(500, 'kind-a')
export const PODS_KIND_B = genPods(40, 'kind-b', 7)

/** The pod US1 tests open in the inspector. */
export const DETAIL_POD: PodSummary = PODS_500[0] as PodSummary

// ------------------------------------------------------------------ system

export const backendStarted: EventBatch = batch([system(1, { type: 'BackendStarted' })])

export const uiConfigDefault: EventBatch = batch([
  system(2, {
    type: 'UiConfigLoaded',
    data: { ui: null, expanded_groups: ['Workloads'], mcp_auto_start: false },
  }),
])

/** A saved config (light theme, compact density) for persistence round-trip tests. */
export const SAVED_UI: UiConfig = {
  schema: 1,
  theme: { mode: 'light', accent: 'magenta', effects: 'off', glitch: false },
  density: { override: 'compact' },
  layouts: {},
  lastSelection: { context: 'kind-a', namespace: 'payments', section: SECTIONS.pods },
}

export const uiConfigSaved: EventBatch = batch([
  system(2, {
    type: 'UiConfigLoaded',
    data: { ui: SAVED_UI, expanded_groups: ['Workloads', 'Network'], mcp_auto_start: true },
  }),
])

/** `RefreshContexts` / `Initialize` with kubeconfig contexts kind-a (current) and kind-b. */
export function contexts(current: FixtureContext | null = 'kind-a', seq = 3): EventBatch {
  return batch([
    system(seq, { type: 'ContextsUpdated', data: { contexts: [...CONTEXTS], current } }),
  ])
}

export const twoContexts: EventBatch = batch([
  ...contexts('kind-a').envelopes,
  system(4, { type: 'ContextChanged', data: { current: 'kind-a' } }),
])

/** `RefreshContexts` without a kubeconfig. */
export const emptyContexts: EventBatch = batch([
  system(3, { type: 'ContextsUpdated', data: { contexts: [], current: null } }),
])

/** No kubeconfig: the backend reports empty contexts, then the connect error verbatim. */
export const noKubeconfig: EventBatch = batch([
  ...emptyContexts.envelopes,
  system(4, {
    type: 'Notification',
    data: { level: 'Info', message: 'Connecting to cluster (kubeconfig/in-cluster)...' },
  }),
  system(5, {
    type: 'Notification',
    data: {
      level: 'Error',
      message:
        'failed to infer config: in-cluster: (failed to read an incluster environment variable: environment variable not found), kubeconfig: (failed to read kubeconfig from "/home/operator/.kube/config": No such file or directory (os error 2))',
    },
  }),
])

export function contextChanged(context: FixtureContext, seq = 10): EventBatch {
  return batch([
    system(seq, { type: 'NamespaceFilterChanged', data: { namespaces: [] } }),
    system(seq + 1, { type: 'ContextChanged', data: { current: context } }),
    ...contexts(context, seq + 2).envelopes,
  ])
}

// ------------------------------------------------------------------ lists

export function namespaces(context: FixtureContext, seq = 20): EventBatch {
  return batch([
    list(seq, clusterRoute(context), { type: 'NamespacesUpdated', data: [...NAMESPACES[context]] }),
  ])
}

export function pods(
  context: FixtureContext,
  rows: PodSummary[] = context === 'kind-a' ? PODS_500 : PODS_KIND_B,
  seq = 30,
): EventBatch {
  return batch([list(seq, nsRoute(context, SECTIONS.pods), { type: 'PodsUpdated', data: rows })])
}

/** `namespaceFilter` mimics the backend's server-side filter (`[]` = all). */
export function deployments(
  context: FixtureContext,
  namespaceFilter: readonly string[] = [],
  seq = 40,
): EventBatch {
  const rows = genDeployments(context).filter(
    (d) => namespaceFilter.length === 0 || namespaceFilter.includes(d.namespace),
  )
  return batch([
    list(seq, nsRoute(context, SECTIONS.deployments), { type: 'DeploymentsUpdated', data: rows }),
  ])
}

export const pods500: EventBatch = pods('kind-a')

/** 10,000 pods in one batch (the constitution V / SC perf target). */
export function pods10k(): EventBatch {
  return pods('kind-a', genPods(10_000, 'kind-a'))
}

export const deploymentsKindA: EventBatch = deployments('kind-a')

/**
 * A late kind-a pods list that arrives after the operator switched to kind-b. It must be
 * dropped (rule 1), so its one pod name never shows up in the kind-b list.
 */
export const STALE_POD_NAME = 'stale-from-kind-a'
export const staleKindA: EventBatch = batch([
  list(90, nsRoute('kind-a', SECTIONS.pods), {
    type: 'PodsUpdated',
    data: [
      {
        name: STALE_POD_NAME,
        namespace: 'default',
        phase: 'Running',
        node: 'kind-a-worker-1',
        ready: 1,
        total: 1,
        restarts: 0,
      },
    ],
  }),
])

/**
 * Request-correlated pod lists for the superseded-request rule (FR-016): request 8 is applied,
 * then the slower reply to request 7 arrives and must be dropped.
 */
export const requestNewer: EventBatch = batch([
  list(
    60,
    nsRoute('kind-a', SECTIONS.pods),
    { type: 'PodsUpdated', data: PODS_500.slice(0, 2) },
    { type: 'Request', data: 8 },
  ),
])
export const requestOlder: EventBatch = batch([
  list(
    61,
    nsRoute('kind-a', SECTIONS.pods),
    { type: 'PodsUpdated', data: PODS_500.slice(0, 5) },
    { type: 'Request', data: 7 },
  ),
])

// ------------------------------------------------------------------ reads

/** `SelectPod` reply: details (raw_json feeds the YAML view). */
export function podDetail(context: FixtureContext, pod: PodSummary, seq = 50): EventBatch {
  return batch([
    read(seq, nsRoute(context, SECTIONS.pods, pod.namespace), {
      type: 'PodDetailsUpdated',
      data: podDetails(pod),
    }),
  ])
}

export const podDetailKindA: EventBatch = podDetail('kind-a', DETAIL_POD)

/** `RefreshEvents` reply: cluster events (global route, no context) about DETAIL_POD. */
export function clusterEvents(pod: PodSummary = DETAIL_POD, seq = 55): EventBatch {
  const events = podEvents(pod)
  return batch([
    list(seq, globalRoute(SECTIONS.events), {
      type: 'EventsUpdated',
      data: { events, total_count: events.length },
    }),
  ])
}

export const podEventsKindA: EventBatch = clusterEvents()

// ------------------------------------------------------------------ errors

/** RBAC failure on a pods refresh, reported verbatim as the backend does (notify_error). */
export const PODS_FORBIDDEN =
  'pods is forbidden: User "system:serviceaccount:dev:viewer" cannot list resource "pods" in API group "" at the cluster scope'

export const podsError: EventBatch = batch([
  system(70, { type: 'Notification', data: { level: 'Error', message: PODS_FORBIDDEN } }),
])

/** Backpressure: the pump dropped envelopes before this batch. */
export const droppedWarning: EventBatch = batch([], 3)
