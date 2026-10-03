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
import {
  CLUSTER_CONTAINERS,
  CLUSTER_CONTAINERS_NODE_DOWN,
  containerReplies,
  manyContainers,
} from './containers'

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
    description:
      'As default, but kind-a has a long chain of pods, each related to the next (044 US2).',
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
  // 048-docker-management: one scenario per awkward acceptance case, because none of them —
  // a socket that refuses, five hundred containers, a definition file that will not parse, a
  // preview that has gone stale — can be scheduled against a real daemon.
  containers: {
    description: 'Runtime ready: three projects, two strays and a kind node, nothing adopted.',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: { ...cluster(), ...containerReplies() },
  },
  'containers-adopted': {
    description: 'As containers, but `shop` is adopted and fathom-dev is Fathom-managed.',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      ...cluster(),
      // `activeContext` is the context the operator is on, so a break of `fathom-dev` from
      // `kind-a` is a break they are *not* connected to — the half of the warning that must not
      // be said when it is not true.
      ...containerReplies({
        adopted: ['shop'],
        managedClusters: ['fathom-dev'],
        activeContext: 'kind-a',
      }),
    },
  },
  // US5: the two awkward halves of logs and a shell. Neither can be scheduled against a real
  // daemon — an image without a shell and a follow that has already dropped lines are both states
  // you have to be handed.
  'containers-no-shell': {
    description: 'As containers-adopted, but shop-web has no /bin/sh for a session to run.',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      ...cluster(),
      ...containerReplies({
        adopted: ['shop'],
        managedClusters: ['fathom-dev'],
        activeContext: 'kind-a',
        noShellIn: 'shop-web',
      }),
    },
  },
  'containers-chatty': {
    description: 'As containers-adopted, but the follow has already dropped earlier lines.',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      ...cluster(),
      ...containerReplies({
        adopted: ['shop'],
        managedClusters: ['fathom-dev'],
        activeContext: 'kind-a',
        truncatedLogs: true,
      }),
    },
  },
  // US4: the cross-link. The operator is on `kind-a`, the context kind gives the cluster the node
  // container backs — so a stop of that node is both a break and a break of the cluster they are
  // connected to, which are two separate sentences.
  'containers-cluster-connected': {
    description: 'Connected to kind-a, whose node is a container on this daemon.',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      ...cluster(),
      ...containerReplies({
        containers: CLUSTER_CONTAINERS,
        managedClusters: ['a'],
        activeContext: 'kind-a',
      }),
    },
  },
  // US4 scenario 4: a node stopped outside Fathom. Neither the container row nor the cluster may
  // read as healthy afterwards.
  'containers-node-stopped': {
    description: 'A kind node container exited behind Fathom’s back.',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      ...cluster(),
      ...containerReplies({
        containers: CLUSTER_CONTAINERS_NODE_DOWN,
        managedClusters: ['a'],
        activeContext: 'kind-a',
      }),
    },
  },
  'containers-absent': {
    description: 'No container runtime on PATH: the scene is absent, not an empty error screen.',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: { ...cluster(), ...containerReplies({ presence: 'absent' }) },
  },
  'containers-stopped': {
    description: 'Runtime installed and not running — said in those words, not "no containers".',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: { ...cluster(), ...containerReplies({ presence: 'stopped' }) },
  },
  'containers-unreachable': {
    description: "The socket refuses: the runtime's own permission text, naming the socket.",
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: { ...cluster(), ...containerReplies({ presence: 'unreachable' }) },
  },
  'containers-lost': {
    description: 'Ready once, then the daemon stops answering: the list goes stale, not blank.',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: { ...cluster(), ...containerReplies({ unavailableAfter: 1 }) },
  },
  'containers-500': {
    description: 'One project with 500 containers, for the render budget at 10k objects.',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: { ...cluster(), ...containerReplies({ containers: manyContainers() }) },
  },
  'containers-corrupt-definition': {
    description: '`shop` declares a definition file that will not parse; the project still lists.',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      ...cluster(),
      ...containerReplies({ corruptDefinition: 'shop', adopted: ['shop'] }),
    },
  },
  'containers-partial-failure': {
    description: '`shop` is adopted; `shop-db` refuses to stop while `shop-web` obeys.',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      ...cluster(),
      ...containerReplies({ adopted: ['shop'], failingMember: 'shop-db' }),
    },
  },
  'containers-vanished-member': {
    description: '`shop-db` is gone by the time the operation is issued: the daemon says so.',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      ...cluster(),
      ...containerReplies({ adopted: ['shop'], vanishedMember: 'shop-db' }),
    },
  },
  'containers-volume-in-use': {
    description: '`shop` is adopted; the daemon refuses to remove the volume shop-web holds.',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      ...cluster(),
      ...containerReplies({ adopted: ['shop'], failingMember: 'in_use_data' }),
    },
  },
  'containers-lying-runtime': {
    description: 'A removal the runtime calls a success, over a volume the next reading still has.',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: {
      ...cluster(),
      ...containerReplies({ adopted: ['shop'], lyingAbout: 'shop_db_data' }),
    },
  },
  'containers-stale-preview': {
    description: 'Every issue is refused as stale, whatever was previewed.',
    initialState: { context: 'kind-a', namespaceFilter: [] },
    onSubscribe: [backendStarted],
    replies: { ...cluster(), ...containerReplies({ adopted: ['shop'], stalePreview: true }) },
  },
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
