/**
 * Named fake-backend scenarios for `pnpm dev:fake` (`?scenario=<name>`) and Playwright
 * (`window.__fathomFake.load(name)`). A scenario is what the backend emits on subscribe plus
 * how it answers each command; unknown commands get no reply (like a slow/idle backend).
 */
import type { BackendCmd, EventBatch, PodSummary } from '../../src/protocol/types'
import {
  backendStarted,
  CASCADE_CHAIN,
  cascadeChainRelated,
  CASCADE_PODS,
  clusterEvents,
  connectionProblem,
  contextChanged,
  contexts,
  contextsWithProblem,
  kubeconfigEmpty,
  kubeconfigMalformed,
  noCurrentContext,
  type ProblemKind,
  deploymentDetail,
  deployments,
  emptyContexts,
  namespaces,
  noKubeconfig,
  PODS_500,
  PODS_KIND_B,
  PODS_FORBIDDEN,
  podDetail,
  pods,
  staleKindA,
  twoContexts,
  uiConfigDefault,
  uiConfigRemembering,
} from './batches'
import { batch, system } from './builders'
import { genDeployments, genPods, type FixtureContext } from './gen-pods'
import {
  exportKubeconfigReply,
  fetchManifestsReply,
  previewKubeconfigReply,
} from './manifest-export'

/** Mutable backend-side state the fake keeps between commands. */
export interface FakeBackendState {
  context: FixtureContext | null
  namespaceFilter: string[]
}

type CmdOf<T extends BackendCmd['type']> = Extract<BackendCmd, { type: T }>
export type Reply<T extends BackendCmd['type'] = BackendCmd['type']> = (
  cmd: CmdOf<T>,
  state: FakeBackendState,
) => EventBatch[]
export type Replies = { [T in BackendCmd['type']]?: Reply<T> }

export interface Scenario {
  description: string
  /** Backend state when the scenario loads. */
  initialState: FakeBackendState
  /** Emitted right after `subscribe`. */
  onSubscribe: EventBatch[]
  replies: Replies
}

function filterPods(rows: PodSummary[], filter: string[]): PodSummary[] {
  return filter.length === 0 ? rows : rows.filter((p) => filter.includes(p.namespace))
}

/**
 * kind-a is current but cannot be used (`kind`): every connect to it reports the same problem
 * and it never lists anything; kind-b works and clears its problem when chosen.
 */
function brokenActiveContext(kind: ProblemKind): Scenario {
  const base = cluster()
  const broken = (s: FakeBackendState) => s.context === 'kind-a'
  const guard =
    <T extends BackendCmd['type']>(reply: Reply<T> | undefined): Reply<T> =>
    (cmd, s) =>
      broken(s) || !reply ? [] : reply(cmd, s)
  return {
    description: `kind-a is current and ${kind}; kind-b connects.`,
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      ...base,
      RefreshContexts: () => [contexts('kind-a')],
      Initialize: () => [contextsWithProblem(kind)],
      SetContext: (cmd, s) => {
        if (cmd.data.context === 'kind-a') {
          s.context = 'kind-a'
          return [connectionProblem('kind-a', kind, 90)]
        }
        const replies = base.SetContext?.(cmd, s) ?? []
        return [connectionProblem(cmd.data.context, null, 60), ...replies]
      },
      RefreshPods: guard(base.RefreshPods),
      RefreshDeployments: guard(base.RefreshDeployments),
      RefreshNamespaces: guard(base.RefreshNamespaces),
      SetActiveSection: guard(base.SetActiveSection),
      RefreshEvents: guard(base.RefreshEvents),
    },
  }
}

/** 043: no current context; picking kind-a on the setup screen fails with `kind`. */
function setupBroken(kind: ProblemKind): Scenario {
  const base = cluster()
  return {
    description: `No current context; kind-a is ${kind} when picked, kind-b connects.`,
    initialState: { context: null, namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      ...base,
      RefreshContexts: () => [contexts(null)],
      Initialize: () => [noCurrentContext],
      SetContext: (cmd, s) => {
        if (cmd.data.context === 'kind-a') {
          s.context = 'kind-a'
          return [contextChanged('kind-a'), connectionProblem('kind-a', kind, 90)]
        }
        return base.SetContext?.(cmd, s) ?? []
      },
    },
  }
}

/** 043: no current context, and Fathom's UI config remembers `context` from an earlier pick. */
function rememberedContext(context: string): Scenario {
  return {
    description: `No current context; the UI config remembers ${context}.`,
    initialState: { context: null, namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      ...cluster(),
      LoadUiConfig: () => [uiConfigRemembering(context)],
      RefreshContexts: () => [contexts(null)],
      Initialize: () => [noCurrentContext],
    },
  }
}

/** A browsable two-context cluster; `podRows` lets a scenario swap in a bigger kind-a list. */
function cluster(podRows: () => PodSummary[] = () => PODS_500): Replies {
  const podsFor = (s: FakeBackendState): EventBatch[] => {
    if (!s.context) return []
    const rows = s.context === 'kind-a' ? podRows() : PODS_KIND_B
    return [pods(s.context, filterPods(rows, s.namespaceFilter))]
  }
  const deploymentsFor = (s: FakeBackendState): EventBatch[] =>
    s.context ? [deployments(s.context, s.namespaceFilter)] : []

  return {
    LoadUiConfig: () => [uiConfigDefault],
    RefreshContexts: (_cmd, s) => [contexts(s.context)],
    Initialize: (_cmd, s) => {
      s.context ??= 'kind-a'
      return [twoContexts, namespaces(s.context), ...podsFor(s), ...deploymentsFor(s)]
    },
    SetContext: (cmd, s) => {
      const next = cmd.data.context
      if (next !== 'kind-a' && next !== 'kind-b') {
        return [
          batch([
            system(80, {
              type: 'Notification',
              data: { level: 'Error', message: `context "${next}" not found in kubeconfig` },
            }),
          ]),
        ]
      }
      const late = s.context === 'kind-a' && next === 'kind-b' ? [staleKindA] : []
      s.context = next
      s.namespaceFilter = []
      return [...late, contextChanged(next), namespaces(next), ...podsFor(s), ...deploymentsFor(s)]
    },
    SetNamespaceFilter: (cmd, s) => {
      s.namespaceFilter = cmd.data.namespaces
      return [
        batch([
          system(25, { type: 'NamespaceFilterChanged', data: { namespaces: cmd.data.namespaces } }),
        ]),
        ...podsFor(s),
        ...deploymentsFor(s),
      ]
    },
    RefreshNamespaces: (_cmd, s) => (s.context ? [namespaces(s.context)] : []),
    RefreshPods: (_cmd, s) => podsFor(s),
    RefreshDeployments: (_cmd, s) => deploymentsFor(s),
    SetActiveSection: (cmd, s) =>
      cmd.data.section === 'workloads/pods'
        ? podsFor(s)
        : cmd.data.section === 'workloads/deployments'
          ? deploymentsFor(s)
          : [],
    SelectPod: (cmd, s) => {
      if (!s.context) return []
      const rows = s.context === 'kind-a' ? podRows() : PODS_KIND_B
      const pod = rows.find((p) => p.namespace === cmd.data.namespace && p.name === cmd.data.name)
      return pod ? [podDetail(s.context, pod)] : []
    },
    SelectDeployment: (cmd, s) => {
      if (!s.context) return []
      const d = genDeployments(s.context).find(
        (x) => x.namespace === cmd.data.namespace && x.name === cmd.data.name,
      )
      const rows = s.context === 'kind-a' ? podRows() : PODS_KIND_B
      return d ? [deploymentDetail(s.context, d, rows)] : []
    },
    RefreshEvents: (_cmd, s) => {
      const first = s.context === 'kind-b' ? PODS_KIND_B[0] : podRows()[0]
      return first && s.context ? [clusterEvents(first, 55, s.context)] : []
    },
    // 040-manifest-export: read-only fetches and the kubeconfig export (see manifest-export.ts).
    FetchManifests: (cmd, s) =>
      fetchManifestsReply(cmd, s.context, s.context === 'kind-b' ? PODS_KIND_B : podRows()),
    PreviewKubeconfigExport: (cmd) => previewKubeconfigReply(cmd),
    ExportKubeconfig: (cmd) => exportKubeconfigReply(cmd),
  }
}

let pods10kCache: PodSummary[] | undefined
const pods10kRows = (): PodSummary[] => (pods10kCache ??= genPods(10_000, 'kind-a'))

const podsForbidden = (): EventBatch[] => [
  batch([system(70, { type: 'Notification', data: { level: 'Error', message: PODS_FORBIDDEN } })]),
]

export const SCENARIOS = {
  default: {
    description: 'Two contexts (kind-a current, kind-b), 500 pods, deployments, pod detail/events.',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: cluster(),
  },
  // 044 US1: one scenario per relevance case, so the row the auto-open must pick (FR-003) is
  // fixed by the fixture rather than by whatever the 500-pod generator happened to produce.
  'cascade-unhealthy': {
    description: 'As default, but kind-a has four pods, the third of them Failed (044 FR-003).',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: cluster(() => [...CASCADE_PODS.unhealthy]),
  },
  'cascade-warning': {
    description: 'As default, but kind-a has three pods, the second and third Pending (044).',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: cluster(() => [...CASCADE_PODS.warning]),
  },
  'cascade-healthy': {
    description: 'As default, but kind-a has two Running pods (044).',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: cluster(() => [...CASCADE_PODS.healthy]),
  },
  'cascade-empty': {
    description: 'As default, but kind-a has no pods at all (044 FR-004).',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: cluster(() => [...CASCADE_PODS.empty]),
  },
  // 044 US2: the drill-in walk. Each pod's related resources name the next one, so one chain of
  // clicks fills and then overflows the cascade at every width class (FR-005, FR-006).
  'cascade-chain': {
    description: 'As default, but kind-a has six pods, each related to the next (044 US2).',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      ...cluster(() => [...CASCADE_CHAIN]),
      RefreshRelatedResources: (cmd, s) =>
        s.context ? [cascadeChainRelated(s.context, cmd.data.namespace ?? '', cmd.data.name)] : [],
    },
  },
  'pods-10k': {
    description: 'As default, but kind-a has 10,000 pods (perf).',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: cluster(pods10kRows),
  },
  'pods-error': {
    description: 'As default, but listing pods fails with an RBAC error (shown verbatim).',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      ...cluster(),
      Initialize: (_cmd, s) => {
        s.context ??= 'kind-a'
        return [
          twoContexts,
          namespaces(s.context),
          ...podsForbidden(),
          deployments(s.context, s.namespaceFilter),
        ]
      },
      RefreshPods: () => podsForbidden(),
    },
  },
  'no-kubeconfig': {
    description: 'No kubeconfig: empty context list and the connect error.',
    initialState: { context: null, namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      LoadUiConfig: () => [uiConfigDefault],
      RefreshContexts: () => [emptyContexts],
      Initialize: () => [noKubeconfig],
    },
  },
  // T126 first-run scenarios (tests/e2e/first-run-states.spec.ts).
  'kubeconfig-malformed': {
    description: 'Kubeconfig is not valid YAML: the Malformed issue, no contexts.',
    initialState: { context: null, namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      LoadUiConfig: () => [uiConfigDefault],
      RefreshContexts: () => [emptyContexts],
      Initialize: () => [kubeconfigMalformed],
    },
  },
  'kubeconfig-empty': {
    description: 'Kubeconfig defines no contexts: the Empty issue.',
    initialState: { context: null, namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      LoadUiConfig: () => [uiConfigDefault],
      RefreshContexts: () => [emptyContexts],
      Initialize: () => [kubeconfigEmpty],
    },
  },
  'no-current-context': {
    description: 'Two contexts, no current-context: pick one, then it browses normally.',
    initialState: { context: null, namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      ...cluster(),
      RefreshContexts: () => [contexts(null)],
      Initialize: () => [noCurrentContext],
    },
  },
  'context-unreachable': brokenActiveContext('Unreachable'),
  // 043 setup screen (tests/e2e/setup-screen.spec.ts, setup-sign-in.spec.ts).
  'setup-unreachable': setupBroken('Unreachable'),
  'setup-unauthorized': setupBroken('Unauthorized'),
  'remembered-context': rememberedContext('kind-b'),
  'remembered-context-removed': rememberedContext('gone-prod'),
  'context-unauthorized': brokenActiveContext('Unauthorized'),
  manual: {
    description: 'Silent backend: only BackendStarted; tests drive everything with emit().',
    initialState: { context: null, namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {},
  },
} satisfies Record<string, Scenario>

export type ScenarioName = keyof typeof SCENARIOS

export function isScenarioName(name: string): name is ScenarioName {
  return Object.hasOwn(SCENARIOS, name)
}
