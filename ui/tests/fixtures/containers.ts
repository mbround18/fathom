/**
 * 048-docker-management fixtures: a scripted container runtime.
 *
 * The fake keeps its own daemon-side state (presence, grants, the last preview) so the awkward
 * acceptance scenarios are reachable without a real daemon: a socket that refuses, five hundred
 * containers, a definition file that cannot be read, and a preview that has gone stale. Every
 * mutation goes through the same two-phase path the backend uses — preview, then issue against a
 * stored `preview_id` — because a fake that lets an issue through without a preview would make the
 * UI's staleness handling untestable.
 *
 * Deviation from T031: these live in `tests/fixtures/` next to `daemon.ts` and `kinds.ts`, which
 * is where every other feature's scripted backend data lives. `tests/support/` holds Playwright's
 * own runtime configuration (worker sizing) and nothing else.
 */
import type {
  ClusterLink,
  Container,
  ContainerInventory,
  EventBatch,
  ObjectOutcome,
  ObjectRef,
  OperationOutcome,
  OperationPreview,
  OperationRefusal,
  OperationRequest,
  OutcomeStatus,
  Project,
  ProjectGroup,
  PublishedPort,
  RemovableObject,
  RuntimePresence,
  RuntimePresenceState,
  StreamEvent,
  UsageSample,
} from '../../src/protocol/types'
import { batch, globalRoute, stream } from './builders'

/** The socket text a permission failure reports, verbatim as the runtime words it (FR-004). */
export const PERMISSION_DENIED =
  'permission denied while trying to connect to the Docker daemon socket at unix:///var/run/docker.sock'

const SOCKET = 'unix:///var/run/docker.sock'

/**
 * What the daemon says about a container it no longer has, verbatim (FR-004, FR-024).
 *
 * Exported so the spec asserts the runtime's own sentence rather than a paraphrase of it: a
 * rewritten error is the thing the operator cannot search for.
 */
export const noSuchContainer = (name: string) =>
  `Error response from daemon: No such container: ${name}`

/**
 * `succeeded` only when every object succeeded, mirroring `OperationOutcome::overall_from`.
 *
 * Written out here rather than hardcoded, because a fake that reports overall success beside a
 * failed object would let the UI pass a test the real backend would fail.
 */
function overallOf(objects: ObjectOutcome[]): OutcomeStatus {
  if (objects.some((o) => o.status === 'failed')) return 'failed'
  if (objects.length > 0 && objects.every((o) => o.status === 'skipped')) return 'skipped'
  return 'succeeded'
}

let seq = 9000
const ev = (data: StreamEvent): EventBatch => batch([stream(seq++, globalRoute(), data)])

// ---------------------------------------------------------------------------
// Object builders
// ---------------------------------------------------------------------------

export const port = (over: Partial<PublishedPort> = {}): PublishedPort => ({
  container_port: 80,
  host_port: 42080,
  host_ip: '0.0.0.0',
  protocol: 'tcp',
  ...over,
})

export function container(over: Partial<Container> = {}): Container {
  const name = over.name ?? 'web'
  return {
    id: over.id ?? `${name}-0000000000000000000000000000000000000000000000000000000000000000`.slice(0, 64),
    name,
    image: 'nginx:1.27',
    state: 'running',
    health: null,
    status: 'Up 3 hours',
    created_at: '2026-09-28T10:00:00Z',
    command: 'nginx -g daemon off;',
    ports: [port()],
    project: null,
    service: null,
    cluster: null,
    ...over,
  }
}

export const projectGroup = (name: string): ProjectGroup => ({ kind: 'project', name })
export const UNOWNED: ProjectGroup = { kind: 'unowned' }

export function project(over: Partial<Project> & { group: ProjectGroup }): Project {
  const containers = over.containers ?? []
  return {
    members: containers.length,
    running: containers.filter((c) => c.state === 'running').length,
    adoption: 'not_adopted',
    definition_file: null,
    definition_state: 'undeclared',
    containers,
    ...over,
  }
}

export const image = (over: Partial<RemovableObject> = {}): RemovableObject => ({
  class: 'image',
  id: 'sha256:aaaa',
  name: 'nginx:1.27',
  size_bytes: 142_000_000,
  tagged: true,
  created_at: '2026-09-01T00:00:00Z',
  project: null,
  referenced_by: [],
  ...over,
})

export const volume = (over: Partial<RemovableObject> = {}): RemovableObject => ({
  class: 'volume',
  id: 'web_data',
  name: 'web_data',
  // Volumes report no size: a prune of volumes whose sizes are unknown must not read as
  // reclaiming nothing (FR-010).
  size_bytes: null,
  tagged: null,
  created_at: '2026-09-01T00:00:00Z',
  project: null,
  referenced_by: [],
  ...over,
})

export const network = (over: Partial<RemovableObject> = {}): RemovableObject => ({
  class: 'network',
  id: 'net-0001',
  name: 'shop_default',
  size_bytes: null,
  tagged: null,
  created_at: '2026-09-01T00:00:00Z',
  project: null,
  referenced_by: [],
  ...over,
})

export const presence = (
  state: RuntimePresenceState,
  over: Partial<RuntimePresence> = {},
): RuntimePresence => ({
  state,
  runtime: state === 'absent' ? null : 'docker',
  endpoint: state === 'absent' ? null : SOCKET,
  detail: state === 'unreachable' ? PERMISSION_DENIED : null,
  ...over,
})

export const AS_OF = '2026-10-02T12:00:00Z'

export function inventory(over: Partial<ContainerInventory> = {}): ContainerInventory {
  return {
    projects: [],
    images: [],
    volumes: [],
    networks: [],
    as_of: AS_OF,
    ...over,
  }
}

// ---------------------------------------------------------------------------
// The default corpus: three projects, two strays, one cluster node
// ---------------------------------------------------------------------------

const shop = [
  container({ name: 'shop-web', project: 'shop', service: 'web' }),
  container({
    name: 'shop-db',
    project: 'shop',
    service: 'db',
    image: 'postgres:16',
    ports: [],
    health: 'healthy',
  }),
]

const blog = [
  container({ name: 'blog-web', project: 'blog', service: 'web' }),
  container({
    name: 'blog-worker',
    project: 'blog',
    service: 'worker',
    state: 'exited',
    status: 'Exited (137) 2 minutes ago',
    ports: [],
  }),
]

const api = [
  container({
    name: 'api-app',
    project: 'api',
    service: 'app',
    state: 'restarting',
    status: 'Restarting (1) 4 seconds ago',
  }),
]

/** Two containers with no compose project at all, plus a kind node that also carries none. */
const strays = [
  container({ name: 'scratch', project: null, ports: [] }),
  container({ name: 'redis-adhoc', project: null, image: 'redis:7', ports: [] }),
  container({
    name: 'fathom-dev-control-plane',
    project: null,
    image: 'kindest/node:v1.31.0',
    cluster: 'fathom-dev',
    ports: [],
  }),
]

export const ALL_CONTAINERS: Container[] = [...shop, ...blog, ...api, ...strays]

/** 500 containers in one project, for the virtualization and render-budget assertions. */
export function manyContainers(count = 500): Container[] {
  return Array.from({ length: count }, (_, i) =>
    container({
      id: `bulk${String(i).padStart(60, '0')}`,
      name: `bulk-${String(i)}`,
      project: 'bulk',
      service: `svc-${String(i % 7)}`,
      state: i % 11 === 0 ? 'exited' : 'running',
      ports: [],
    }),
  )
}

function groupsOf(containers: Container[], opts: ScriptOptions, adopted: string[]): Project[] {
  const byProject = new Map<string | null, Container[]>()
  for (const c of containers) {
    byProject.set(c.project, [...(byProject.get(c.project) ?? []), c])
  }
  // Sorted, and the no-project bucket last: a list that reshuffles between refreshes makes two
  // readings incomparable.
  const named = [...byProject.keys()].filter((k): k is string => k !== null).sort()
  const groups = named.map((name) =>
    project({
      group: projectGroup(name),
      containers: byProject.get(name) ?? [],
      adoption: adopted.includes(name) ? 'adopted' : 'not_adopted',
      definition_file: definitionFileOf(name),
      definition_state:
        opts.corruptDefinition === name
          ? { unreadable: 'mapping values are not allowed in this context at line 7 column 9' }
          : opts.missingDefinition === name
            ? 'missing'
            : 'present',
    }),
  )
  const unowned = byProject.get(null)
  return unowned ? [...groups, project({ group: UNOWNED, containers: unowned })] : groups
}

function linksOf(containers: Container[], managed: string[]): ClusterLink[] {
  const byCluster = new Map<string, string[]>()
  for (const c of containers) {
    if (c.cluster) byCluster.set(c.cluster, [...(byCluster.get(c.cluster) ?? []), c.id])
  }
  return [...byCluster.entries()].map(([cluster, container_ids]) => ({
    cluster,
    container_ids,
    managed_by_fathom: managed.includes(cluster),
  }))
}

// ---------------------------------------------------------------------------
// Single event batches, for tests that drive the store directly
// ---------------------------------------------------------------------------

export const presenceUpdated = (state: RuntimePresenceState, over?: Partial<RuntimePresence>) =>
  ev({ type: 'ContainerRuntimePresenceUpdated', data: { presence: presence(state, over) } })

export const inventoryUpdated = (inv: ContainerInventory) =>
  ev({ type: 'ContainerInventoryUpdated', data: inv })

export const inventoryUnavailable = (reason: string) =>
  ev({ type: 'ContainerInventoryUnavailable', data: { reason } })

export const adoptionUpdated = (adopted: string[], fell_back = false) =>
  ev({ type: 'ContainerAdoptionUpdated', data: { adopted, fell_back } })

export const operationPreviewed = (request_id: string, preview: OperationPreview) =>
  ev({ type: 'ContainerOperationPreviewed', data: { request_id, preview } })

export const operationFinished = (request_id: string, outcome: OperationOutcome) =>
  ev({ type: 'ContainerOperationFinished', data: { request_id, outcome } })

export const operationRefused = (request_id: string, reason: OperationRefusal) =>
  ev({ type: 'ContainerOperationRefused', data: { request_id, reason } })

export const clusterLinksUpdated = (links: ClusterLink[]) =>
  ev({ type: 'ContainerClusterLinksUpdated', data: { links } })

export const usageSampled = (samples: UsageSample[], at_cap = false) =>
  ev({ type: 'ContainerUsageSampled', data: { samples, at_cap } })

export const logBatch = (stream_id: number, lines: string[], truncated = false) =>
  ev({ type: 'ContainerLogBatch', data: { stream_id, lines, truncated } })

export const sessionClosed = (stream_id: number, error: string | null) =>
  ev({ type: 'ContainerSessionClosed', data: { stream_id, error } })

// ---------------------------------------------------------------------------
// The scripted backend
// ---------------------------------------------------------------------------

export interface ScriptOptions {
  /** Presence the fake reports. Defaults to `ready`. */
  presence?: RuntimePresenceState
  /** Containers the daemon holds. Defaults to [`ALL_CONTAINERS`]. */
  containers?: Container[]
  /** Projects already adopted when the scene opens. */
  adopted?: string[]
  /** Clusters Fathom provisioned, so their node containers are actionable without a grant. */
  managedClusters?: string[]
  /** This project's definition file is present but unparseable. */
  corruptDefinition?: string
  /** This project's definition file is declared and gone. */
  missingDefinition?: string
  /**
   * Every issue is refused as stale, whatever was previewed — the inventory moving under a
   * preview is otherwise a race the test cannot schedule.
   */
  stalePreview?: boolean
  /** The daemon stops answering after this many inventory refreshes (1 = the second one fails). */
  unavailableAfter?: number
  /**
   * This container, by name, fails when the operation reaches it. Its siblings succeed — which is
   * the case FR-022 exists for: one argv, several objects, and not one answer for all of them.
   */
  failingMember?: string
  /**
   * This container, by name, is gone from the daemon by the time the operation is issued. The
   * preview named it, so the outcome must say what became of it in the runtime's own words
   * rather than quietly dropping it from the report.
   */
  vanishedMember?: string
}

/** Mutable state the fake keeps between commands, the way a daemon would. */
interface Daemon {
  adopted: string[]
  previews: Map<string, OperationPreview>
  refreshes: number
}

/**
 * The argv a request runs, derived here exactly once.
 *
 * The UI asserts against this, so it is written as the literal argv a reader could paste into a
 * shell — not a prose rendering of one (FR-018).
 */
export function argvFor(request: OperationRequest, containers: Container[]): string[] {
  switch (request.kind) {
    case 'lifecycle':
      // One argv naming every member, exactly as the backend builds it — never one command per
      // container. That is what lets FR-029's one preview and one confirmation hold.
      return ['docker', request.action, ...targetsFor(request, containers).map((t) => t.id)]
    case 'remove':
      return ['docker', request.objects[0]?.class ?? 'container', 'rm', ...request.objects.map((o) => o.id)]
    case 'prune_selection':
      return ['docker', request.class, 'prune', '--force']
    case 'create_container':
      return ['docker', 'run', '--detach', '--name', request.spec.name ?? 'unnamed', request.spec.image]
    case 'build_image':
      return ['docker', 'build', '--tag', request.spec.tag, request.spec.context_path]
    case 'create_network':
      return ['docker', 'network', 'create', request.spec.name]
    case 'create_volume':
      return ['docker', 'volume', 'create', request.spec.name]
    case 'project_lifecycle':
      return [
        'docker',
        'compose',
        '-p',
        request.project,
        '-f',
        definitionFileOf(request.project),
        request.action,
        // Detached, because an `up` that held the terminal open would hold a backend task open
        // with it and there is no terminal for the operator to read.
        ...(request.action === 'up' ? ['-d'] : []),
      ]
  }
}

/** Where the fake daemon says a project's file lives. One spelling, used by argv and by groups. */
function definitionFileOf(project: string): string {
  return `/home/op/${project}/compose.yaml`
}

function objectRefOf(c: Container): ObjectRef {
  return { class: 'container', id: c.id, name: c.name }
}

/**
 * Every object the argv names, in argv order.
 *
 * Sorted by name rather than by whatever order the daemon listed them in, so a preview read twice
 * reads the same twice, and so the per-object outcomes line up with what the operator read.
 */
export function targetsFor(request: OperationRequest, containers: Container[]): ObjectRef[] {
  const membersOf = (project: string) =>
    containers
      .filter((c) => c.project === project)
      .sort((a, b) => a.name.localeCompare(b.name))
      .map(objectRefOf)
  switch (request.kind) {
    case 'lifecycle': {
      // Bound first: narrowing does not survive into the filter's closure.
      const scope = request.scope
      return scope.kind === 'container'
        ? containers.filter((c) => c.id === scope.id).map(objectRefOf)
        : membersOf(scope.project)
    }
    case 'project_lifecycle':
      return membersOf(request.project)
    case 'remove':
      return request.objects
    default:
      return []
  }
}

function previewFor(
  request: OperationRequest,
  destructive: boolean,
  containers: Container[],
): OperationPreview {
  const removals =
    request.kind === 'remove'
      ? request.objects.map((object) => ({ object, size_bytes: null, project: null }))
      : []
  return {
    preview_id: `pv-${String(Math.abs(hash(JSON.stringify(request))))}`,
    request,
    command: argvFor(request, containers),
    destructive,
    removals,
    total_size_bytes: null,
    blast_radius: [],
    excluded: [],
    breaks_clusters: [],
    required_confirmation: destructive ? 'remove' : null,
    as_of: AS_OF,
  }
}

function hash(s: string): number {
  let h = 0
  for (let i = 0; i < s.length; i += 1) h = (h * 31 + s.charCodeAt(i)) | 0
  return h
}

const DESTRUCTIVE: OperationRequest['kind'][] = ['remove', 'prune_selection']

/**
 * The container-runtime half of a scenario's `replies`, as a spreadable fragment.
 *
 * Typed loosely at the boundary because `Replies` is keyed by the whole command union; the
 * handlers themselves are narrowed by the command's own `kind`.
 */
export function containerReplies(opts: ScriptOptions = {}) {
  const containers = opts.containers ?? ALL_CONTAINERS
  const managed = opts.managedClusters ?? []
  const daemon: Daemon = {
    adopted: [...(opts.adopted ?? [])],
    previews: new Map(),
    refreshes: 0,
  }
  const state = opts.presence ?? 'ready'

  const refuse = (request_id: string, reason: OperationRefusal) => [
    operationRefused(request_id, reason),
  ]

  return {
    RefreshContainerRuntimePresence: () => [presenceUpdated(state)],

    RefreshContainerInventory: () => {
      daemon.refreshes += 1
      if (state === 'unreachable') return [inventoryUnavailable(PERMISSION_DENIED)]
      if (state === 'stopped')
        return [inventoryUnavailable('The container runtime is installed but not running.')]
      if (state === 'absent')
        return [inventoryUnavailable('No container runtime was found on this machine.')]
      if (opts.unavailableAfter !== undefined && daemon.refreshes > opts.unavailableAfter)
        return [inventoryUnavailable(PERMISSION_DENIED)]
      return [
        // The backend emits the grants the first time it touches them, which is this same
        // command (`RuntimeState::adoption`). Leaving them out here would let the UI pass on an
        // inventory snapshot the real backend never ships alone.
        adoptionUpdated([...daemon.adopted]),
        inventoryUpdated(
          inventory({
            projects: groupsOf(containers, opts, daemon.adopted),
            images: [image(), image({ id: 'sha256:bbbb', name: '<none>:<none>', tagged: false })],
            volumes: [volume()],
            networks: [network()],
          }),
        ),
        clusterLinksUpdated(linksOf(containers, managed)),
      ]
    },

    LoadProjectDefinition: () => [],

    AdoptContainerProject: (cmd: { data: { project: string } }) => {
      if (!daemon.adopted.includes(cmd.data.project)) daemon.adopted.push(cmd.data.project)
      return [adoptionUpdated([...daemon.adopted])]
    },

    ReleaseContainerProject: (cmd: { data: { project: string } }) => {
      const at = daemon.adopted.indexOf(cmd.data.project)
      // A release with no grant is refused, not silently honoured — that is also what keeps an
      // implicit, cluster-derived grant from looking revocable.
      if (at < 0) return refuse(`release:${cmd.data.project}`, { kind: 'not_adopted', detail: cmd.data.project })
      daemon.adopted.splice(at, 1)
      return [adoptionUpdated([...daemon.adopted])]
    },

    PreviewContainerOperation: (cmd: {
      data: { request_id: string; request: OperationRequest }
    }) => {
      const preview = previewFor(
        cmd.data.request,
        DESTRUCTIVE.includes(cmd.data.request.kind),
        containers,
      )
      daemon.previews.set(preview.preview_id, preview)
      // A preview is free on an unadopted project: it is how the operator decides whether to
      // adopt it, and refusing here would make adoption a blind choice.
      return [operationPreviewed(cmd.data.request_id, preview)]
    },

    IssueContainerOperation: (cmd: {
      data: { request_id: string; preview_id: string; typed_confirmation: string | null }
    }) => {
      const preview = daemon.previews.get(cmd.data.preview_id)
      if (!preview || opts.stalePreview) return refuse(cmd.data.request_id, { kind: 'preview_stale' })
      if (preview.required_confirmation && cmd.data.typed_confirmation !== preview.required_confirmation)
        return refuse(cmd.data.request_id, { kind: 'confirmation_mismatch' })
      const project = projectOfRequest(preview.request)
      if (project !== null && !daemon.adopted.includes(project))
        return refuse(cmd.data.request_id, { kind: 'not_adopted', detail: project })
      // Per object, because one argv covering four containers is still four answers. The
      // outcomes are attributed to the targets the preview named, so an object that vanished is
      // still accounted for instead of disappearing from the report with it.
      const objects: ObjectOutcome[] = (
        preview.removals.length > 0
          ? preview.removals.map((r) => r.object)
          : targetsFor(preview.request, containers)
      ).map((object) => {
        if (object.name === opts.vanishedMember)
          return { object, status: 'failed' as const, error: noSuchContainer(object.name) }
        if (object.name === opts.failingMember)
          return {
            object,
            status: 'failed' as const,
            error: `Error response from daemon: cannot stop container ${object.name}: permission denied`,
          }
        return { object, status: 'succeeded' as const, error: null }
      })
      return [
        ev({
          type: 'ContainerOperationFinished',
          data: {
            request_id: cmd.data.request_id,
            outcome: {
              preview_id: preview.preview_id,
              // Byte-identical to the preview's argv. The fake re-reads it rather than
              // rebuilding it, for the same reason the backend does.
              command: preview.command,
              overall: overallOf(objects),
              objects,
              // A member that vanished between preview and issue is a difference the post-check
              // found, and it is reported rather than swallowed (FR-024).
              discrepancies: objects
                .filter((o) => o.object.name === opts.vanishedMember)
                .map((o) => ({ object: o.object, expected: 'running', observed: 'gone' })),
            },
          },
        }),
      ]
    },

    StartContainerLogStream: (cmd: { data: { stream_id: number; container_id: string } }) => [
      logBatch(cmd.data.stream_id, [
        `2026-10-02T12:00:00Z starting ${cmd.data.container_id.slice(0, 12)}`,
        '2026-10-02T12:00:01Z listening on :80',
      ]),
    ],
    StopContainerLogStream: () => [],

    StartContainerSession: (cmd: { data: { stream_id: number; container_id: string } }) => {
      const owner = containers.find((c) => c.id === cmd.data.container_id)
      // Implicit adoption is decided per container: a node of a cluster Fathom manages is
      // actionable without a grant, and it carries no compose project to grant.
      const implicit = owner?.cluster != null && managed.includes(owner.cluster)
      const granted = implicit || (owner?.project != null && daemon.adopted.includes(owner.project))
      // A refused session closes the pane that is waiting for it; an operation refusal would
      // arrive nowhere the pane is looking.
      if (!granted)
        return [
          sessionClosed(
            cmd.data.stream_id,
            `'${owner?.project ?? cmd.data.container_id}' has not been adopted, so Fathom will not change it.`,
          ),
        ]
      return [ev({ type: 'ContainerSessionOutput', data: { stream_id: cmd.data.stream_id, data: '/ # ' } })]
    },
    StopContainerSession: (cmd: { data: { stream_id: number } }) => [
      sessionClosed(cmd.data.stream_id, null),
    ],
    SendContainerSessionInput: (cmd: { data: { stream_id: number; data: string } }) => [
      ev({
        type: 'ContainerSessionOutput',
        data: { stream_id: cmd.data.stream_id, data: cmd.data.data },
      }),
    ],
    ResizeContainerSession: () => [],

    StartContainerUsageSampling: (cmd: { data: { container_ids: string[] } }) => [
      usageSampled(
        cmd.data.container_ids.map((container_id, i) => ({
          container_id,
          at: AS_OF,
          cpu_percent: 1.5 + i,
          memory_bytes: 64_000_000 + i * 1_000_000,
          memory_limit_bytes: 2_000_000_000,
        })),
      ),
    ],
    StopContainerUsageSampling: () => [],
  }
}

/** Which project a request's grant hangs on, or `null` when it needs none. */
function projectOfRequest(request: OperationRequest): string | null {
  switch (request.kind) {
    case 'lifecycle':
      return request.scope.kind === 'project' ? request.scope.project : null
    case 'project_lifecycle':
      return request.project
    default:
      return null
  }
}
