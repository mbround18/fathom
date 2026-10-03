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
  ExcludedObject,
  ObjectClass,
  ObjectOutcome,
  ObjectRef,
  OperationOutcome,
  OperationPreview,
  OperationRefusal,
  OperationRequest,
  OutcomeStatus,
  Project,
  ProjectGroup,
  PruneFilter,
  PublishedPort,
  RemovableObject,
  RemovalLine,
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
 * What the daemon says about a volume a container still holds, verbatim (FR-004).
 *
 * Exported for the same reason [`noSuchContainer`] is: the spec asserts the runtime's sentence,
 * not a paraphrase, because a rewritten error is the one the operator cannot search for.
 */
export const volumeInUse = (name: string, holder: string) =>
  `Error response from daemon: remove ${name}: volume is in use - [${holder}]`

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
    id:
      over.id ??
      `${name}-0000000000000000000000000000000000000000000000000000000000000000`.slice(0, 64),
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

// ---------------------------------------------------------------------------
// Removable objects, and what a removal of them would actually take
//
// The backend never hands a prune to the daemon as a prune: `blast_radius.rs` resolves one into
// the identities it covers right now, and what runs is a removal of exactly those. The fake
// resolves it the same way, because a fixture that previewed `docker image prune --force` would
// let the UI pass while the product it is standing in for had been fixed years ago.
// ---------------------------------------------------------------------------

/** The three removable-object pools one reading of the daemon holds. */
export interface ObjectPools {
  images: RemovableObject[]
  volumes: RemovableObject[]
  networks: RemovableObject[]
}

/**
 * The default object corpus, shaped so a shared daemon's two rules are visible in it:
 *
 * - `sha256:cccc` is untagged and owned by nobody, but `blog` reaches it through a stopped
 *   container. While `blog` is unadopted, a dangling-image prune must leave it alone —
 *   reachability decides, not ownership (FR-025).
 * - `sha256:dddd` carries `tagged: null`: the runtime did not say. A prune of the images it could
 *   classify must not sweep up the one it could not.
 * - `shop_db_data` and both networks declare a project, so the same prune that reclaims
 *   `web_data` excludes them until that project is adopted.
 */
export const DEFAULT_OBJECTS: ObjectPools = {
  images: [
    image(),
    image({ id: 'sha256:bbbb', name: '<none>:<none>', tagged: false, size_bytes: 31_000_000 }),
    image({
      id: 'sha256:cccc',
      name: '<none>:<none>',
      tagged: false,
      size_bytes: 88_000_000,
      referenced_by: [
        { kind: 'stopped_container', id: 'blog-worker-id', name: 'blog-worker', project: 'blog' },
      ],
    }),
    image({ id: 'sha256:dddd', name: 'unknown-provenance', tagged: null, size_bytes: 12_000_000 }),
  ],
  volumes: [
    volume(),
    volume({ id: 'shop_db_data', name: 'shop_db_data', project: 'shop' }),
    // Held by a running container, so no prune of the unused ones reaches it and its row says who
    // has it. Owned by nobody, yet `shop` reaches it — which is what decides whether it may go.
    volume({
      id: 'in_use_data',
      name: 'in_use_data',
      referenced_by: [
        { kind: 'running_container', id: 'shop-web-id', name: 'shop-web', project: 'shop' },
      ],
    }),
  ],
  networks: [
    network({ project: 'shop' }),
    network({ id: 'net-0002', name: 'blog_default', project: 'blog' }),
  ],
}

/** One reading of the runtime, which is what every removal set is resolved against. */
export interface Scene extends ObjectPools {
  containers: Container[]
  adopted: string[]
}

/** A resolved removal set: what goes, what stays, and whose work it reaches. */
export interface Radius {
  removals: RemovalLine[]
  projects: string[]
  excluded: ExcludedObject[]
}

/**
 * Why an operation cannot be described in advance.
 *
 * Carries the operator-facing sentence, not a code, because that is what crosses the wire: the
 * backend answers an unpreviewable request with `OperationRefusal::NotPreviewable(detail)` against
 * the request's own id, so the reason lands on the control that asked. The constructors below
 * mirror `NotPreviewable::detail` in Rust word for word — a fixture that paraphrases is a fixture
 * the UI can pass against wording the product does not ship.
 */
export class NotPreviewable extends Error {
  /** The scope cannot be enumerated from this inventory. */
  static notEnumerable(what: string) {
    return new NotPreviewable(
      `Fathom cannot list in advance everything ${what} would affect, so it does not offer it.`,
    )
  }

  /** The operation names something this inventory does not contain. */
  static unknownTarget(id: string) {
    return new NotPreviewable(`'${id}' is not in the inventory Fathom last read.`)
  }

  /** Every object in the selection was excluded, so there is nothing left to remove. */
  static allExcluded(projects: string[]) {
    return new NotPreviewable(
      `Every object in that selection belongs to a project that has not been adopted: ${projects.join(
        ', ',
      )}. Adopt one to act on its objects.`,
    )
  }
}

const POOL_OF: Record<Exclude<ObjectClass, 'container'>, keyof ObjectPools> = {
  image: 'images',
  volume: 'volumes',
  network: 'networks',
}

function poolOf(scene: Scene, objectClass: ObjectClass): RemovableObject[] {
  return objectClass === 'container' ? [] : scene[POOL_OF[objectClass]]
}

/**
 * The class a filter can possibly select.
 *
 * A filter and a class that disagree are not a narrower selection, they are a mistake, and
 * resolving one of them silently would remove the wrong kind of object.
 */
function classOfFilter(filter: PruneFilter): ObjectClass {
  switch (filter) {
    case 'dangling_images':
    case 'unused_images':
      return 'image'
    case 'unused_volumes':
      return 'volume'
    case 'unused_networks':
      return 'network'
    case 'stopped_containers':
      return 'container'
  }
}

/** An object resolved out of the inventory, with everything the adoption rule needs. */
interface Candidate {
  object: ObjectRef
  size_bytes: number | null
  /** The project the object declares, if any. */
  project: string | null
  /** Every project that can reach it, including the one it declares. */
  reachableFrom: string[]
}

const candidateOfObject = (o: RemovableObject): Candidate => ({
  object: { class: o.class, id: o.id, name: o.name },
  size_bytes: o.size_bytes,
  project: o.project,
  reachableFrom: [
    ...(o.project === null ? [] : [o.project]),
    ...o.referenced_by.map((r) => r.project).filter((p): p is string => p !== null),
  ],
})

const candidateOfContainer = (c: Container): Candidate => ({
  object: objectRefOf(c),
  // The runtime reports a container's writable-layer size only when asked separately, and an
  // unasked-for number is not a number. `null`, never `0`.
  size_bytes: null,
  project: c.project,
  reachableFrom: c.project === null ? [] : [c.project],
})

/**
 * Applies the adoption rule to a resolved selection.
 *
 * One pass, so an object cannot land in both lists: the first unadopted project that can reach it
 * decides, and the reason names that project rather than saying "a project" — the operator's next
 * move is to adopt it, and they need to know which it is.
 */
function judge(scene: Scene, candidates: Candidate[]): Radius {
  const removals: RemovalLine[] = []
  const excluded: ExcludedObject[] = []
  const projects = new Set<string>()
  for (const candidate of candidates) {
    const blocker = candidate.reachableFrom.find((p) => !scene.adopted.includes(p))
    if (blocker !== undefined) {
      excluded.push({
        object: candidate.object,
        reason: { kind: 'unadopted_project', detail: blocker },
      })
      continue
    }
    for (const reached of candidate.reachableFrom) projects.add(reached)
    removals.push({
      object: candidate.object,
      size_bytes: candidate.size_bytes,
      project: candidate.project,
    })
  }
  // A selection every object of which an unadopted project can reach has nothing left in it, and
  // is not offered as a removal of nothing: a typed confirmation over a no-op teaches the operator
  // that the guard is noise, and the sentence they can act on names the project to adopt.
  if (removals.length === 0)
    throw NotPreviewable.allExcluded([...new Set(excluded.map((e) => e.reason.detail))].sort())
  return { removals, projects: [...projects].sort(), excluded }
}

function locate(scene: Scene, want: ObjectRef): Candidate {
  if (want.class === 'container') {
    const held = scene.containers.find((c) => c.id === want.id)
    if (!held) throw NotPreviewable.unknownTarget(want.id)
    return candidateOfContainer(held)
  }
  const held = poolOf(scene, want.class).find((o) => o.id === want.id)
  if (!held) throw NotPreviewable.unknownTarget(want.id)
  return candidateOfObject(held)
}

/**
 * Resolves an explicit list of objects the operator picked.
 *
 * An id this inventory does not contain is refused rather than passed through: the operator is
 * looking at a list, and an object that is not in it means the list is older than they think.
 */
export function resolveObjects(scene: Scene, objects: ObjectRef[]): Radius {
  if (objects.length === 0) throw NotPreviewable.notEnumerable('removing nothing at all')
  return judge(
    scene,
    objects.map((want) => locate(scene, want)),
  )
}

/**
 * Resolves a prune filter into the identities it covers, right now.
 *
 * That is the whole point: `docker image prune` run a second later is a different operation from
 * the one previewed, and this one cannot be (FR-026).
 */
export function resolvePrune(scene: Scene, objectClass: ObjectClass, filter: PruneFilter): Radius {
  if (classOfFilter(filter) !== objectClass)
    throw NotPreviewable.notEnumerable('a prune whose filter and object class disagree')
  const candidates =
    filter === 'stopped_containers'
      ? scene.containers
          .filter((c) => c.state === 'exited' || c.state === 'dead')
          .map(candidateOfContainer)
      : filter === 'dangling_images'
        ? // `tagged: null` means the runtime did not say. An image Fathom cannot classify is not
          // swept up by a prune of the ones it can.
          scene.images.filter((i) => i.tagged === false).map(candidateOfObject)
        : poolOf(scene, objectClass)
            .filter((o) => o.referenced_by.length === 0)
            .map(candidateOfObject)
  // An operation whose scope cannot be enumerated is not offered at all (FR-027).
  if (candidates.length === 0) throw NotPreviewable.notEnumerable('a prune with nothing in it')
  return judge(scene, candidates)
}

/** The argv a resolved removal set runs: by identity, never a filter. */
export function removalArgv(removals: RemovalLine[]): string[] {
  const classes = [...new Set(removals.map((r) => r.object.class))]
  const [only] = classes
  if (classes.length !== 1 || only === undefined)
    throw NotPreviewable.notEnumerable('removing objects of more than one kind in one command')
  const ids = removals.map((r) => r.object.id)
  // A container is removed by `rm` alone; the others carry their class first.
  return only === 'container' ? ['docker', 'rm', ...ids] : ['docker', only, 'rm', ...ids]
}

const PLURAL: Record<ObjectClass, string> = {
  container: 'containers',
  image: 'images',
  volume: 'volumes',
  network: 'networks',
}

/**
 * The string the operator types to confirm a removal.
 *
 * The object's own name when there is one, the count and the kind when there are several. Either
 * way it changes with what is being removed, which is the point: a confirmation that reads the
 * same every time is a reflex, and a reflex approves the wrong list as readily as the right one.
 */
export function confirmationFor(removals: RemovalLine[]): string {
  const [first] = removals
  if (removals.length === 1 && first) return first.object.name
  return `remove ${String(removals.length)} ${PLURAL[first?.object.class ?? 'container']}`
}

/**
 * Total of the sizes that are known.
 *
 * `null` when nothing in the set reports one, rather than `0`: "we do not know how much this
 * frees" and "this frees nothing" are different sentences, and only one of them makes a removal
 * look pointless (FR-010).
 */
function sumSizes(removals: RemovalLine[]): number | null {
  const known = removals.map((r) => r.size_bytes).filter((s): s is number => s !== null)
  return known.length === 0 ? null : known.reduce((a, b) => a + b, 0)
}

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
  /**
   * Images, volumes and networks the daemon holds, merged over [`DEFAULT_OBJECTS`].
   *
   * One pool, read by the inventory the operator sees *and* by the resolver that decides what a
   * removal takes — so a preview can never describe a set the list on screen does not contain.
   */
  objects?: Partial<ObjectPools>
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
  /**
   * This object, by name, is reported removed while the next reading still shows it.
   *
   * Runtimes do lie: a removal can come back successful against a daemon whose state did not
   * change. FR-023's post-verification exists for precisely this, and without a fixture that lies
   * there is nothing to verify against.
   */
  lyingAbout?: string
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
export function argvFor(
  request: OperationRequest,
  containers: Container[],
  removals: RemovalLine[] = [],
): string[] {
  switch (request.kind) {
    case 'lifecycle':
      // One argv naming every member, exactly as the backend builds it — never one command per
      // container. That is what lets FR-029's one preview and one confirmation hold.
      return ['docker', request.action, ...targetsFor(request, containers).map((t) => t.id)]
    // Both are resolved by identity before they reach here, and the argv is built from the
    // resolved set — never from the filter, and never as a prune (FR-026).
    case 'remove':
    case 'prune_selection':
      return removalArgv(removals)
    case 'create_container':
      return [
        'docker',
        'run',
        '--detach',
        '--name',
        request.spec.name ?? 'unnamed',
        request.spec.image,
      ]
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

function radiusFor(request: OperationRequest, scene: Scene): Radius | null {
  if (request.kind === 'remove') return resolveObjects(scene, request.objects)
  if (request.kind === 'prune_selection') return resolvePrune(scene, request.class, request.filter)
  return null
}

function previewFor(request: OperationRequest, scene: Scene): OperationPreview {
  const base = {
    preview_id: `pv-${String(Math.abs(hash(JSON.stringify(request))))}`,
    request,
    destructive: isDestructive(request),
    breaks_clusters: [],
    as_of: AS_OF,
  }
  // One resolution fills the list, the radius, the exclusions and the confirmation, so none of
  // the four can disagree with the others about the same selection.
  const radius = radiusFor(request, scene)
  if (radius) {
    return {
      ...base,
      command: removalArgv(radius.removals),
      removals: radius.removals,
      total_size_bytes: sumSizes(radius.removals),
      blast_radius: radius.projects,
      excluded: radius.excluded,
      required_confirmation: confirmationFor(radius.removals),
    }
  }
  // What a `down` takes with it: the project's containers, every one named. Volumes are absent on
  // purpose — `down` leaves them unless asked with `-v`, and listing them would warn about a loss
  // that is not going to happen.
  const down =
    request.kind === 'project_lifecycle' && request.action === 'down' ? request.project : null
  const removals: RemovalLine[] =
    down === null
      ? []
      : targetsFor(request, scene.containers).map((object) => ({
          object,
          size_bytes: null,
          project: down,
        }))
  return {
    ...base,
    command: argvFor(request, scene.containers),
    removals,
    total_size_bytes: null,
    blast_radius: down === null ? [] : [down],
    excluded: [],
    // The project's own name, not a constant: a confirmation that is the same word every time is
    // a reflex, and a reflex confirms the wrong project as readily as the right one.
    required_confirmation: down,
  }
}

function hash(s: string): number {
  let h = 0
  for (let i = 0; i < s.length; i += 1) h = (h * 31 + s.charCodeAt(i)) | 0
  return h
}

/** Which requests carry the full guard, matching `OperationRequest::is_destructive`. */
function isDestructive(request: OperationRequest): boolean {
  if (request.kind === 'remove' || request.kind === 'prune_selection') return true
  return request.kind === 'project_lifecycle' && request.action === 'down'
}

/**
 * The container-runtime half of a scenario's `replies`, as a spreadable fragment.
 *
 * Typed loosely at the boundary because `Replies` is keyed by the whole command union; the
 * handlers themselves are narrowed by the command's own `kind`.
 */
export function containerReplies(opts: ScriptOptions = {}) {
  const containers = opts.containers ?? ALL_CONTAINERS
  const objects: ObjectPools = { ...DEFAULT_OBJECTS, ...opts.objects }
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
            // The same pool the resolver reads, so the preview can only ever describe objects the
            // operator is actually looking at.
            ...objects,
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
      if (at < 0)
        return refuse(`release:${cmd.data.project}`, {
          kind: 'not_adopted',
          detail: cmd.data.project,
        })
      daemon.adopted.splice(at, 1)
      return [adoptionUpdated([...daemon.adopted])]
    },

    PreviewContainerOperation: (cmd: {
      data: { request_id: string; request: OperationRequest }
    }) => {
      let preview: OperationPreview
      try {
        preview = previewFor(cmd.data.request, { ...objects, containers, adopted: daemon.adopted })
      } catch (e) {
        // An operation whose scope cannot be enumerated is not offered at all — and it is not an
        // `OperationRefusal` either, so it arrives as the notification the backend sends (FR-027).
        if (!(e instanceof NotPreviewable)) throw e
        // Correlated by request id, so the panel the operator is looking at says why rather than a
        // sentence appearing in the status bar belonging to no control in particular.
        return [
          ev({
            type: 'ContainerOperationRefused',
            data: {
              request_id: cmd.data.request_id,
              reason: { kind: 'not_previewable', detail: e.message },
            },
          }),
        ]
      }
      daemon.previews.set(preview.preview_id, preview)
      // A preview is free on an unadopted project: it is how the operator decides whether to
      // adopt it, and refusing here would make adoption a blind choice.
      return [operationPreviewed(cmd.data.request_id, preview)]
    },

    IssueContainerOperation: (cmd: {
      data: { request_id: string; preview_id: string; typed_confirmation: string | null }
    }) => {
      const preview = daemon.previews.get(cmd.data.preview_id)
      if (!preview || opts.stalePreview)
        return refuse(cmd.data.request_id, { kind: 'preview_stale' })
      if (
        preview.required_confirmation &&
        cmd.data.typed_confirmation !== preview.required_confirmation
      )
        return refuse(cmd.data.request_id, { kind: 'confirmation_mismatch' })
      const project = projectOfPreview(preview)
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
            // The daemon's refusal is a different sentence for a volume than for a container, and
            // reporting one of them in the other's words is the paraphrase FR-004 forbids.
            error:
              object.class === 'volume'
                ? volumeInUse(object.name, 'shop-web-id')
                : `Error response from daemon: cannot stop container ${object.name}: permission denied`,
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
              discrepancies: [
                // A member that vanished between preview and issue is a difference the post-check
                // found, and it is reported rather than swallowed (FR-024).
                ...objects
                  .filter((o) => o.object.name === opts.vanishedMember)
                  .map((o) => ({ object: o.object, expected: 'running', observed: 'gone' })),
                // The other direction: a removal the runtime called a success, over an object the
                // next reading still holds. Worded exactly as `mutate::verify` words it.
                ...objects
                  .filter((o) => o.object.name === opts.lyingAbout)
                  .map((o) => ({
                    object: o.object,
                    expected: 'gone',
                    observed: 'still present',
                  })),
              ],
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
      return [
        ev({
          type: 'ContainerSessionOutput',
          data: { stream_id: cmd.data.stream_id, data: '/ # ' },
        }),
      ]
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

/**
 * Which project a preview's grant hangs on, or `null` when it needs none.
 *
 * For a removal it is the one project every object in the set shares — the resolver already
 * excluded anything an unadopted project could reach, so a set that shares none belongs to no
 * project at all, and demanding a grant over the no-project bucket would make a dangling image
 * impossible to ever remove.
 */
function projectOfPreview(preview: OperationPreview): string | null {
  const [first, ...rest] = preview.removals
  if (first) {
    const shared = first.project
    return shared !== null && rest.every((r) => r.project === shared) ? shared : null
  }
  return projectOfRequest(preview.request)
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
