/**
 * Recorded-shape `EventBatch` fixtures for US1 (contracts/ipc.md § Verification).
 *
 * Routes and correlations mirror what `src/backend` emits today: System events on
 * `Route::global("system")`, namespaces on `Route::cluster("system", ctx)`, pod/deployment lists
 * on `Route::namespaced(section, ctx, None)`, reads on the object's namespace, and cluster
 * events on `Route::cluster("events", ctx)`. Protocol v1 backends send `Correlation::None`; the
 * `Request` fixtures below exercise the staleness rule for when they don't.
 */
import type { EventBatch, PodSummary, UiConfig, WorkloadSummary } from '../../src/protocol/types'
import { batch, clusterRoute, list, nsRoute, read, SECTIONS, system } from './builders'
import {
  deploymentDetails,
  genDeployments,
  genPods,
  NAMESPACES,
  podDetails,
  podDetailsWithLifecycle,
  podEvents,
  podEventsWithLifecycle,
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
    data: { ui: null, expanded_groups: ['Workloads'], mcp_stdio_enabled: false },
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
    data: { ui: SAVED_UI, expanded_groups: ['Workloads', 'Network'], mcp_stdio_enabled: true },
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

/** No kubeconfig: empty contexts, the classified `Missing` issue, and no active context. The
 * backend no longer attempts a connect (nor notifies a raw error) when there is nothing to
 * connect to. */
export const NO_KUBECONFIG_PATH = '/home/operator/.kube/config'
export const noKubeconfig: EventBatch = batch([
  ...emptyContexts.envelopes,
  system(4, {
    type: 'KubeconfigStatus',
    data: { issue: { kind: 'Missing', paths: [NO_KUBECONFIG_PATH], detail: '' } },
  }),
  system(5, { type: 'ContextChanged', data: { current: null } }),
])

// ------------------------------------------------------------------ first run (T126)

export const MALFORMED_DETAIL = 'did not find expected key at line 4 column 3'

/** A kubeconfig that is not valid YAML. */
export const kubeconfigMalformed: EventBatch = batch([
  ...emptyContexts.envelopes,
  system(4, {
    type: 'KubeconfigStatus',
    data: { issue: { kind: 'Malformed', paths: [NO_KUBECONFIG_PATH], detail: MALFORMED_DETAIL } },
  }),
  system(5, { type: 'ContextChanged', data: { current: null } }),
])

/** A kubeconfig with no contexts in it (an empty file or `contexts: []`). */
export const kubeconfigEmpty: EventBatch = batch([
  ...emptyContexts.envelopes,
  system(4, {
    type: 'KubeconfigStatus',
    data: { issue: { kind: 'Empty', paths: [NO_KUBECONFIG_PATH], detail: '' } },
  }),
  system(5, { type: 'ContextChanged', data: { current: null } }),
])

/** Contexts exist but `current-context` is unset. */
export const noCurrentContext: EventBatch = batch([
  ...contexts(null).envelopes,
  system(4, {
    type: 'KubeconfigStatus',
    data: { issue: { kind: 'NoCurrentContext', paths: [NO_KUBECONFIG_PATH], detail: '' } },
  }),
  system(5, { type: 'ContextChanged', data: { current: null } }),
])

export const UNREACHABLE_SERVER = 'https://127.0.0.1:1'
export const UNREACHABLE_DETAIL =
  'HyperError: error trying to connect: tcp connect error: Connection refused (os error 111)'
export const UNAUTHORIZED_DETAIL =
  'ApiError: Unauthorized: Unauthorized (ErrorResponse { status: "Failure", message: "Unauthorized", reason: "Unauthorized", code: 401 })'

export type ProblemKind = 'Unreachable' | 'Unauthorized'

/** `ActiveConnectionProblem` for `context` (`null` kind = it connected). */
export function connectionProblem(context: string, kind: ProblemKind | null, seq = 7): EventBatch {
  return batch([
    system(seq, {
      type: 'ActiveConnectionProblem',
      data: {
        context,
        problem:
          kind === null
            ? null
            : {
                kind,
                server: UNREACHABLE_SERVER,
                detail: kind === 'Unauthorized' ? UNAUTHORIZED_DETAIL : UNREACHABLE_DETAIL,
              },
      },
    }),
  ])
}

/** kind-a is current but cannot be used: status Error plus a classified problem. */
export function contextsWithProblem(kind: ProblemKind): EventBatch {
  return batch([
    ...contexts('kind-a').envelopes,
    system(4, { type: 'KubeconfigStatus', data: { issue: null } }),
    system(5, {
      type: 'ContextStatusUpdated',
      data: {
        context: 'kind-a',
        status: 'Error',
        error: kind === 'Unauthorized' ? UNAUTHORIZED_DETAIL : UNREACHABLE_DETAIL,
        plugin_error: null,
      },
    }),
    ...connectionProblem('kind-a', kind, 6).envelopes,
    system(7, { type: 'ContextChanged', data: { current: 'kind-a' } }),
  ])
}

/**
 * 012-incluster-daemon-federation P1: a third context beyond `CONTEXTS`, unreachable in the
 * fixture data. Kept separate from `FixtureContext` (`kind-a`/`kind-b`) since the fake backend's
 * `SCENARIOS` never connects to it — it exists purely to exercise the always-visible status
 * strip's `error` state.
 */
export const FEDERATION_ERROR_CONTEXT = 'kind-c'
export const FEDERATION_ERROR_MESSAGE =
  'Timed out after 5s probing context: dial tcp 10.0.0.9:6443: i/o timeout'

/**
 * `ContextsUpdated` (3 contexts) + `ContextStatusUpdated` for each (US1 e2e: FR-001/FR-002/FR-004).
 * `kind-a` is `Connected` (mirrors the active-context connect bracketing in
 * `src/backend/cmd/cluster.rs`), `kind-b` is `NotConnected` (a settled probe), `kind-c` is
 * `Error` with a distinctive, verbatim-checkable message.
 */
export const federationContexts: EventBatch = batch([
  system(3, {
    type: 'ContextsUpdated',
    data: { contexts: ['kind-a', 'kind-b', FEDERATION_ERROR_CONTEXT], current: 'kind-a' },
  }),
  system(4, { type: 'ContextChanged', data: { current: 'kind-a' } }),
])

export const federationStatuses: EventBatch = batch([
  system(5, {
    type: 'ContextStatusUpdated',
    data: { context: 'kind-a', status: 'Connected', error: null, plugin_error: null },
  }),
  system(6, {
    type: 'ContextStatusUpdated',
    data: { context: 'kind-b', status: 'NotConnected', error: null, plugin_error: null },
  }),
  system(7, {
    type: 'ContextStatusUpdated',
    data: {
      context: FEDERATION_ERROR_CONTEXT,
      status: 'Error',
      error: FEDERATION_ERROR_MESSAGE,
      plugin_error: null,
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

/**
 * 008-pod-lifecycle-diagnostics: `SelectPod` reply with a rich `containers_lifecycle` (init
 * containers, probes, restart history, hooks) for the `us{1..5}-pod-lifecycle-*` e2e specs.
 */
export function podLifecycleDetail(context: FixtureContext, pod: PodSummary, seq = 50): EventBatch {
  return batch([
    read(seq, nsRoute(context, SECTIONS.pods, pod.namespace), {
      type: 'PodDetailsUpdated',
      data: podDetailsWithLifecycle(pod),
    }),
  ])
}

/** `SelectPod` reply after a debug container was added (025): one ephemeral entry, `waiting`. */
export function podDetailWithDebugContainer(
  context: FixtureContext,
  pod: PodSummary,
  name: string,
  image: string,
  seq = 60,
): EventBatch {
  const base = podDetails(pod)
  return batch([
    read(seq, nsRoute(context, SECTIONS.pods, pod.namespace), {
      type: 'PodDetailsUpdated',
      data: {
        ...base,
        containers_lifecycle: [
          ...base.containers_lifecycle,
          {
            name,
            is_init: false,
            is_ephemeral: true,
            image,
            ready: false,
            state: {
              phase: 'waiting',
              reason: null,
              exit_code: null,
              started_at: null,
              finished_at: null,
            },
            restart_count: 0,
            last_state: null,
            probes: [],
            hooks: [],
          },
        ],
      },
    }),
  ])
}

/** Cluster events batch matching `podLifecycleDetail`'s probe/hook failure fixtures. */
export function podLifecycleEvents(
  pod: PodSummary,
  seq = 51,
  context: FixtureContext = 'kind-a',
): EventBatch {
  const events = podEventsWithLifecycle(pod)
  return batch([
    list(seq, clusterRoute(context, SECTIONS.events), {
      type: 'EventsUpdated',
      data: { events, total_count: events.length },
    }),
  ])
}

/** `SelectDeployment` reply (raw_json feeds the YAML view). */
export function deploymentDetail(
  context: FixtureContext,
  d: WorkloadSummary,
  rows: readonly PodSummary[] = context === 'kind-a' ? PODS_500 : PODS_KIND_B,
  seq = 52,
): EventBatch {
  return batch([
    read(seq, nsRoute(context, SECTIONS.deployments, d.namespace), {
      type: 'DeploymentDetailsUpdated',
      data: deploymentDetails(d, rows),
    }),
  ])
}

/** `RefreshEvents` reply: cluster events (`Route::cluster("events", ctx)`) about `pod`. */
export function clusterEvents(
  pod: PodSummary = DETAIL_POD,
  seq = 55,
  context: string = 'kind-a',
): EventBatch {
  const events = podEvents(pod)
  return batch([
    list(seq, clusterRoute(context, SECTIONS.events), {
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

/** 019: healthy cluster version — server and every kubelet match (no skew). */
export const clusterVersionHealthy: EventBatch = batch([
  system(20, {
    type: 'ClusterVersionUpdated',
    data: { server_version: 'v1.29.4', node_kubelet_versions: ['v1.29.4'], skew_detected: false },
  }),
])

/** 019: a node's kubelet lags the control plane (skew). */
export const clusterVersionSkewed: EventBatch = batch([
  system(21, {
    type: 'ClusterVersionUpdated',
    data: {
      server_version: 'v1.29.4',
      node_kubelet_versions: ['v1.28.9', 'v1.29.4'],
      skew_detected: true,
    },
  }),
])

/** 019: version discovery failed — neutral unknown state. */
export const clusterVersionUnknown: EventBatch = batch([
  system(22, {
    type: 'ClusterVersionUpdated',
    data: { server_version: null, node_kubelet_versions: [], skew_detected: false },
  }),
])
