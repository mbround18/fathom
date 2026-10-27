/**
 * The fake Depths backend every Playwright story runs against (T043).
 *
 * It is installed on a page with Playwright's network interception, so the app under test is the
 * real production bundle and holds no fake mode of its own:
 *
 * - `GET /api/v1/me` answers the scripted `Me`, or a 401 naming the configured providers.
 * - `POST /auth/logout` and `/auth/refresh` are recorded and answered; `/auth/login` is a stub page.
 * - `GET /api/v1/audit/export?…` is `audit`, filtered, as JSONL (admin only, 403 otherwise).
 * - Spec 053: `GET /api/v1/stacks/{stack}/files/download` and `POST …/files/upload` answer from the
 *   volume model in `files` (fake-files.ts), which also answers the file commands on the socket.
 * - Spec 052: `/api/v1/networks*`, `/api/v1/sessions*` and the admin `/api/v1/users/{id}/sessions|networks*`
 *   answer from `networks`, `sessionList`, `userSessions` and `userNetworks`. While `me.network` is
 *   untrusted, every command and route outside the restricted list is refused `untrusted_network`
 *   and audited `network_refused`, as the server does (FR-009..FR-011).
 * - `/api/v1/ws` is a mocked WebSocket that speaks the generated protocol: it checks `Hello`, closes
 *   with the contract's codes, and answers each `CmdEnvelope` with `EventEnvelope`s.
 *
 * Every command is authorised against the scripted session as it is *at the time of the command*,
 * as the real server does (FR-032, FR-033), so a story can change grants mid-test.
 *
 * Usage, in a spec:
 *
 * ```ts
 * import { expect, fixtures as f, test } from '../../support/fake-backend'
 *
 * test('lists stacks', async ({ page, backend }) => {
 *   backend.stacks = f.stackList([f.stack('web'), f.stack('db', { status: 'stopped' })])
 *   backend.on('Confirm', () => [{ type: 'Denied', data: { reason: 'not_permitted' } }])
 *   await page.goto('/')
 *   await backend.waitForCmd('ListStacks')
 *   backend.push({ type: 'StacksChanged', data: backend.stacks })
 * })
 * ```
 *
 * Nothing here sleeps or reads the wall clock for behaviour: output arrives when a command does or
 * when the test pushes it, so stories stay deterministic.
 */
import { test as base, expect, type Page, type Request, type WebSocketRoute } from '@playwright/test'

import type {
  AnswerNetwork,
  AuditEntry,
  AuditFilter,
  Cmd,
  CmdEnvelope,
  ComposeWarning,
  Container,
  ContainerSecurity,
  DenyReason,
  DraftFile,
  Event,
  EventEnvelope,
  FileLimits,
  FilesMode,
  Grant,
  Hello,
  HostSample,
  Me,
  NetworkView,
  OperationKind,
  OpOutcome,
  Preview,
  PreviewVoidReason,
  Provider,
  Role,
  SessionView,
  Stack,
  StackDetail,
  StackList,
  StackOp,
  Step,
  StreamCloseReason,
  TrustDecision,
  UsageSample,
  User,
  UserList,
  WhitelistNetwork,
} from '../../src/protocol/generated'
import { CloseCode, PROTOCOL_VERSION } from '../../src/protocol/generated/constants'
import { FakeFiles, FILE_LIMITS, fakeMount, type FileHttpReply } from './fake-files'

export { expect }
export { DIR_PAGE_SIZE, FakeFiles, FILE_LIMITS, mountId, type RefusalTarget } from './fake-files'

export type CmdType = Cmd['type']
export type CmdOf<T extends CmdType> = Extract<Cmd, { type: T }>

/** What a scripted handler returns: events to send in reply, or nothing to fall through. */
export type Reply = Event[] | undefined

/** Context a handler gets beside the command. */
export interface HandlerContext {
  backend: FakeBackend
  socket: FakeSocket
  id: number
}

export type Handler<T extends CmdType> = (cmd: CmdOf<T>, ctx: HandlerContext) => Reply

/** One HTTP request the app made to the fake, as recorded. */
export interface HttpCall {
  method: string
  path: string
  csrf: string | null
}

/** A scripted operation run: what `Confirm` streams, and how it ends. */
export interface OpScript {
  output: string[]
  outcome: OpOutcome
  /** Stop after the output; `finishOp()` ends it. Lets a story see the running state, or `Busy`. */
  hold: boolean
}

/** One open mocked socket. */
export class FakeSocket {
  readonly hellos: unknown[] = []
  readonly received: CmdEnvelope[] = []
  ready = false
  closed: number | null = null

  private readonly route: WebSocketRoute

  constructor(route: WebSocketRoute) {
    this.route = route
  }

  send(event: Event, replyTo?: number): void {
    if (this.closed !== null) return
    const env: EventEnvelope = replyTo === undefined ? { event } : { reply_to: replyTo, event }
    this.route.send(JSON.stringify(env))
  }

  close(code: number, reason = ''): void {
    if (this.closed !== null) return
    this.closed = code
    void this.route.close({ code, reason })
  }
}

interface HeldOp {
  operationId: string
  stack: string
  kind: OperationKind
  socket: FakeSocket
  replyTo: number
}

interface OpenStream {
  socket: FakeSocket
  kind: 'logs' | 'shell'
}

const NOW = '2026-10-23T12:00:00Z'

/** What an untrusted network may still run (spec 052 FR-009); kept apart from the app's copy on purpose. */
const RESTRICTED_OPS: ReadonlySet<OperationKind> = new Set<OperationKind>([
  'up',
  'stop',
  'restart',
  'service_up',
  'service_stop',
  'service_restart',
])

/** Canned data builders. Each takes overrides for the fields a story cares about. */
export const fixtures = {
  me(over: Partial<Me> = {}): Me {
    return {
      identity: { provider: 'github', subject: '1001', username: 'ada', email: null },
      csrf_token: 'csrf-fake-1',
      role: 'admin',
      owner: false,
      stacks: [],
      features: { shell: true, edit: true, host_metrics: false, files: 'full' },
      limits: { files: { ...FILE_LIMITS } },
      network: { trusted: true, reason: 'first', network: '203.0.113.7/32' },
      needs_answer: false,
      proxy_warning: false,
      ...over,
    }
  },

  /**
   * 053: `me` with the volume explorer in `mode`, and `limits` over the defaults (`FILE_LIMITS`).
   * `fixtures.filesMe('read_only', { editor_max: 1024 })`.
   */
  filesMe(mode: FilesMode, limits: Partial<FileLimits> = {}, over: Partial<Me> = {}): Me {
    const base = fixtures.me(over)
    return { ...base, features: { ...base.features, files: mode }, limits: { files: { ...FILE_LIMITS, ...limits } } }
  },

  /** 053: a ready, writable named volume `app` mounts at `/data`; `backend.files.addMount` seeds one. */
  mount: fakeMount,

  /** A remembered network (spec 052), unknown and untrusted unless `over` says otherwise. */
  networkView(key: string, over: Partial<NetworkView> = {}): NetworkView {
    return {
      key,
      first_used: NOW,
      last_used: NOW,
      days: [NOW.slice(0, 10)],
      source: 'seen',
      answer: 'none',
      trusted: false,
      reason: 'unknown',
      location: 'unknown',
      current: false,
      ...over,
    }
  },

  /** One live session, as `GET /api/v1/sessions` lists it. */
  sessionView(id_hint: string, over: Partial<SessionView> = {}): SessionView {
    return {
      id_hint,
      created: NOW,
      last_used: NOW,
      address: '203.0.113.7',
      network: '203.0.113.7/32',
      trusted: true,
      location: 'unknown',
      browser: 'Firefox on Linux',
      current: false,
      ...over,
    }
  },

  container(over: Partial<Container> = {}): Container {
    const name = over.name ?? 'web-app-1'
    return {
      id: `${name.replace(/[^a-z0-9]/g, '').padEnd(12, '0')}${'0'.repeat(52)}`,
      name,
      image: 'nginx:1.29',
      state: 'running',
      health: null,
      status: 'Up 3 hours',
      created_at: NOW,
      command: '"/docker-entrypoint.sh nginx -g \'daemon off;\'"',
      ports: [],
      project: null,
      service: null,
      cluster: null,
      adoption: 'adopted',
      ...over,
    }
  },

  /** A stack whose single service `app` has one container matching `status`. */
  stack(name: string, over: Partial<Stack> = {}): Stack {
    const status = over.status ?? 'running'
    const running = status === 'running' || status === 'partial'
    const containers = status === 'not_deployed' || status === 'invalid' ? [] : [
      fixtures.container({
        name: `${name}-app-1`,
        project: name,
        service: 'app',
        state: running ? 'running' : 'exited',
        status: running ? 'Up 3 hours' : 'Exited (0) 1 hour ago',
      }),
    ]
    return {
      name,
      valid_name: status !== 'invalid',
      compose_file: 'compose.yaml',
      shadowed_files: [],
      has_env: false,
      content_hash: `hash-${name}-1`,
      status,
      unhealthy: false,
      services: [{ name: 'app', image: 'nginx:1.29', containers }],
      running_services: running ? 1 : 0,
      total_services: 1,
      modified_at: NOW,
      read_error: null,
      invalid_reason: null,
      read_only_reason: null,
      too_large: false,
      ...over,
    }
  },

  stackList(stacks: Stack[], over: Partial<StackList> = {}): StackList {
    return {
      root: '/srv/stacks',
      stacks,
      foreign: [],
      standalone: [],
      warnings: [],
      stale: false,
      as_of: NOW,
      ...over,
    }
  },

  detail(stack: Stack, over: Partial<StackDetail> = {}): StackDetail {
    return {
      stack,
      compose_text: `services:\n  app:\n    image: nginx:1.29\n`,
      env: stack.has_env ? { kind: 'masked', text: 'PORT=8080\nDB_PASSWORD=********\n' } : null,
      content_hash: stack.content_hash,
      ...over,
    }
  },

  user(id: string, over: Partial<User> = {}): User {
    const [provider = 'github', subject = id] = id.split(':') as [Provider?, string?]
    return {
      id,
      provider,
      issuer: null,
      subject,
      username: subject,
      email: null,
      role: 'user',
      stacks: [],
      owner: false,
      pinned_from: null,
      first_seen: NOW,
      last_seen: NOW,
      ...over,
    }
  },

  sample(container_id: string, over: Partial<UsageSample> = {}): UsageSample {
    return {
      container_id,
      at: NOW,
      cpu_percent: 1.5,
      memory_bytes: 64 * 1024 * 1024,
      memory_limit_bytes: 512 * 1024 * 1024,
      ...over,
    }
  },

  /** 25% CPU on 4 cores, 2 of 8 GiB memory, 40 of 100 GiB disk, load 1.00, up a day. */
  hostSample(over: Partial<HostSample> = {}): HostSample {
    const GiB = 1024 ** 3
    return {
      at: NOW,
      cpu_percent: 25,
      cpu_cores: 4,
      load: { one: 1, five: 0.5, fifteen: 0.25 },
      memory: { total_bytes: 8 * GiB, available_bytes: 6 * GiB, used_bytes: 2 * GiB },
      swap: { total_bytes: GiB, free_bytes: GiB, used_bytes: 0 },
      uptime_secs: 86_400,
      disk: { total_bytes: 100 * GiB, used_bytes: 40 * GiB, free_bytes: 60 * GiB },
      containers_running: 3,
      containers_stopped: 1,
      ...over,
    }
  },
}

/** The argv `PreviewOp` shows for each kind, as research R2 lays it out (shape, not exact flags). */
/**
 * A stand-in for the server's compose lint (051 FR-015): `privileged: true` and a mounted docker
 * socket are escapes. Enough for stories; the real rules are tested in depths-core.
 */
export function fakeLint(text: string): ComposeWarning[] {
  const out: ComposeWarning[] = []
  let service = ''
  text.split('\n').forEach((raw, i) => {
    const m = /^ {2}([A-Za-z0-9_.-]+):\s*$/.exec(raw)
    if (m?.[1]) service = m[1]
    const line = i + 1
    if (/^\s+privileged:\s*true\s*$/.test(raw)) {
      out.push({ service, code: 'privileged', severity: 'escape', message: 'runs privileged: it can reach the host', line })
    }
    if (raw.includes('/var/run/docker.sock')) {
      out.push({ service, code: 'runtime_socket', severity: 'escape', message: 'mounts the container runtime socket: it can control the host', line })
    }
  })
  return out
}

function stepsFor(stack: string, op: StackOp): Step[] {
  const compose = ['docker', 'compose', '-p', stack, '-f', 'compose.yaml']
  const cwd = `/srv/stacks/${stack}`
  const step = (...args: string[]): Step => ({ argv: [...compose, ...args], cwd, stdin: 'none' })
  switch (op.kind) {
    case 'up':
      return [step('up', '-d', '--remove-orphans')]
    case 'stop':
      return [step('stop')]
    case 'restart':
      return [step('restart')]
    case 'down':
      return [
        step(
          'down',
          ...(op.options.remove_volumes ? ['--volumes'] : []),
          ...(op.options.remove_images ? ['--rmi', 'all'] : []),
        ),
      ]
    case 'pull':
      return [step('pull')]
    case 'update':
      return [step('pull'), step('up', '-d', '--remove-orphans')]
    case 'delete':
      return [step('down')]
    case 'service_up':
      return [step('up', '-d', op.options.service)]
    case 'service_stop':
      return [step('stop', op.options.service)]
    case 'service_restart':
      return [step('restart', op.options.service)]
  }
}

function grantOf(u: User): Grant {
  return { role: u.role, stacks: u.stacks }
}

/** The stack a command names, if any, for its audit entry. */
function stackOf(cmd: Cmd): string | null {
  switch (cmd.type) {
    case 'GetStack':
    case 'RevealEnv':
      return cmd.data.name
    case 'PreviewOp':
      return cmd.data.stack
    case 'PreviewSave':
      return cmd.data.draft.stack
    case 'OpenLogs':
      return cmd.data.target.kind === 'stack' ? cmd.data.target.stack : null
    case 'ListMounts':
    case 'ListDir':
    case 'ReadFile':
    case 'FolderSize':
    case 'PreviewFileOp':
    case 'ListHistory':
    case 'ReadVersion':
      return cmd.data.stack
    default:
      return null
  }
}

/** Roles `PreviewOp` kinds need beyond an assigned stack (data-model "Operation"). */
function adminOnly(op: StackOp): boolean {
  return op.kind === 'delete' || (op.kind === 'down' && op.options.remove_volumes)
}

function isDestructive(op: StackOp): boolean {
  return op.kind === 'delete' || (op.kind === 'down' && (op.options.remove_volumes || op.options.remove_images))
}

/** Plain unified diff of two texts: every old line removed, every new line added. Enough for a fake. */
function naiveDiff(path: string, before: string, after: string): string {
  const a = before.split('\n').filter((l, i, all) => l !== '' || i < all.length - 1)
  const b = after.split('\n').filter((l, i, all) => l !== '' || i < all.length - 1)
  const body = [...a.filter((l) => !b.includes(l)).map((l) => `-${l}`), ...b.filter((l) => !a.includes(l)).map((l) => `+${l}`)]
  return [`--- a/${path}`, `+++ b/${path}`, `@@ -1,${String(a.length)} +1,${String(b.length)} @@`, ...body].join('\n') + '\n'
}

/**
 * The scriptable backend. Public fields are the scripted state: assign them before `page.goto`,
 * or mid-test (then `push` a `StacksChanged`/`UsersChanged` if the UI should hear about it).
 */
export class FakeBackend {
  /** `null` = no session: `/me` is a 401 and the socket closes 4401. */
  me: Me | null = fixtures.me()
  /** What the 401 body names as configured providers. */
  providers: Provider[] = ['github', 'discord']
  /** Every stack on disk, unfiltered; `ListStacks` filters it for a `user`. */
  stacks: StackList = fixtures.stackList([])
  /** `GetStack` replies; a stack with none gets `fixtures.detail(stack)`. */
  details: Record<string, StackDetail> = {}
  /** Plain `.env` text `RevealEnv` returns, by stack. */
  env: Record<string, string> = {}
  /** What `Confirm` streams. */
  op: OpScript = { output: ['Container app-1  Started\n'], outcome: { kind: 'succeeded' }, hold: false }
  /** Log history by target: a stack name, `stack/service`, or a container ref. */
  logs: Record<string, string[]> = {}
  /** What a new shell prints first (raw text; sent as base64). */
  shellBanner = '/ # '
  /** `StatsSample` sent once per `WatchStats`; empty = one canned sample per targeted container. */
  samples: UsageSample[] = []
  /** `HostSample` sent once per `WatchHost` an admin sends with `me.features.host_metrics` on. */
  host: HostSample = fixtures.hostSample()
  users: UserList = { users: [], pending: [] }
  /** Oldest first, as recorded; `GetAudit` pages it newest first. */
  audit: AuditEntry[] = []
  /** Spec 052: the signed-in person's networks and sessions, and other people's for admins. */
  networks: NetworkView[] = []
  sessionList: SessionView[] = []
  userSessions: Record<string, SessionView[]> = {}
  userNetworks: Record<string, NetworkView[]> = {}
  /** When a network answered "home" now would mature. */
  maturesAt = '2026-10-30T12:00:00Z'
  auditPageSize = 100
  /** Server-side compose errors `PreviewSave` returns for a stack, if any. */
  /** Per stack; an error without `file` is about the compose file. */
  /** 051: what `LintCompose` answers for a stack; unset, a tiny stand-in lint runs. */
  lint: Record<string, ComposeWarning[]> = {}
  /** 051 `DEPTHS_COMPOSE_POLICY`: under `enforce`, escape findings block a user and need an admin's acknowledgement. */
  composePolicy: 'warn' | 'enforce' = 'warn'
  validationErrors: Record<string, { file?: DraftFile; line: number | null; message: string }[]> = {}
  /** Close code for every new socket's `Hello`, e.g. `CloseCode.TooMany`. `null` = accept. */
  helloClose: number | null = null
  /** 053: every stack's mounts and their files; seed with `files.addMount`, `files.write`, … */
  readonly files: FakeFiles = new FakeFiles(
    () => this.me,
    () => `pv-${String(++this.seq)}`,
  )

  readonly sockets: FakeSocket[] = []
  readonly http: HttpCall[] = []

  private readonly handlers = new Map<CmdType, Handler<CmdType>>()
  private readonly waiters: { type: CmdType; resolve: (c: Cmd) => void }[] = []
  /** Commands `waitForCmd` has already handed out. */
  private readonly awaited = new WeakSet<Cmd>()
  private readonly previews = new Map<string, { preview: Preview; used: boolean; expired: boolean }>()
  private readonly held: HeldOp[] = []
  private readonly streams = new Map<number, OpenStream>()
  private seq = 0

  /** Installs the fake on `page`. Call before `page.goto`. */
  static async install(page: Page, setup?: (b: FakeBackend) => void): Promise<FakeBackend> {
    const b = new FakeBackend()
    setup?.(b)
    await b.attach(page)
    return b
  }

  async attach(page: Page): Promise<void> {
    // Later routes take precedence: the catch-all first, so nothing reaches a real server.
    await page.route('**/api/**', (r) => r.fulfill({ status: 404, json: { error: 'not_found' } }))
    await page.route('**/auth/**', (r) => this.auth(r.request()).then((res) => r.fulfill(res)))
    await page.route('**/api/v1/me', (r) => r.fulfill(this.meResponse(r.request())))
    await page.route('**/api/v1/audit/export**', (r) => r.fulfill(this.exportResponse(r.request())))
    await page.route('**/api/v1/stacks/*/files/**', (r) => r.fulfill(this.filesResponse(r.request())))
    for (const glob of ['**/api/v1/networks**', '**/api/v1/sessions**', '**/api/v1/users/**']) {
      await page.route(glob, (r) => r.fulfill(this.networkResponse(r.request())))
    }
    await page.routeWebSocket(/\/api\/v1\/ws$/, (ws) => {
      this.accept(ws)
    })
  }

  // --- Scripting ----------------------------------------------------------------------------

  /** Replaces the default handling of one command. Return `undefined` to fall through to it. */
  on<T extends CmdType>(type: T, handler: Handler<T>): this {
    this.handlers.set(type, handler as unknown as Handler<CmdType>)
    return this
  }

  /** Every `type` command gets `Denied{reason}`. */
  deny(type: CmdType, reason: DenyReason = 'not_permitted'): this {
    return this.on(type, () => [{ type: 'Denied', data: { reason } }])
  }

  /** Sends a live event (no `reply_to`) on every open socket. */
  push(event: Event): void {
    for (const s of this.open()) s.send(event)
  }

  /** Closes every open socket with `code` (e.g. `CloseCode.Unauthenticated` for an expired session). */
  closeSockets(code: number = CloseCode.Unauthenticated): void {
    for (const s of this.open()) s.close(code)
  }

  /** Every command received, across sockets, in order. */
  get received(): Cmd[] {
    return this.sockets.flatMap((s) => s.received.map((e) => e.cmd))
  }

  /** Every `Hello` frame received, across sockets. */
  get hellos(): unknown[] {
    return this.sockets.flatMap((s) => s.hellos)
  }

  /** Resolves with the next `type` command (or one already received and not yet awaited). */
  waitForCmd<T extends CmdType>(type: T): Promise<CmdOf<T>> {
    const seen = this.received.find((c): c is CmdOf<T> => c.type === type && !this.awaited.has(c))
    if (seen) {
      this.awaited.add(seen)
      return Promise.resolve(seen)
    }
    return new Promise((resolve) => {
      this.waiters.push({ type, resolve: resolve as (c: Cmd) => void })
    })
  }

  /** The ids of the held operations, oldest first (ids share a counter with previews and streams). */
  heldOps(): string[] {
    return this.held.map((h) => h.operationId)
  }

  /** Ends a held operation (`op.hold`), sending its `OpFinished`. */
  finishOp(outcome: OpOutcome = this.op.outcome, operationId?: string): void {
    const i = operationId === undefined ? 0 : this.held.findIndex((h) => h.operationId === operationId)
    const [h] = i < 0 ? [] : this.held.splice(i, 1)
    if (!h) throw new Error('fake-backend: no held operation to finish')
    h.socket.send({ type: 'OpFinished', data: { operation_id: h.operationId, outcome } }, h.replyTo)
  }

  /** Marks every outstanding preview expired, as five minutes passing would (file previews too). */
  expirePreviews(): void {
    for (const p of this.previews.values()) p.expired = true
    this.files.expirePreviews()
  }

  /** 053: sets `me.features.files` and merges `limits` into `me.limits.files`, mid-test or before `goto`. */
  setFiles(mode: FilesMode, limits: Partial<FileLimits> = {}): this {
    const me = this.me ?? fixtures.me()
    this.me = { ...me, features: { ...me.features, files: mode }, limits: { files: { ...(me.limits?.files ?? FILE_LIMITS), ...limits } } }
    return this
  }

  /** Sends more log lines on every open log stream (or one, by id). */
  pushLogs(lines: string[], streamId?: number): void {
    for (const [id, s] of this.streams) {
      if (s.kind === 'logs' && (streamId === undefined || streamId === id)) {
        s.socket.send({ type: 'LogChunk', data: { stream_id: id, lines } })
      }
    }
  }

  /** Ends open streams (all, or one) with `reason`. `disconnected` is the proxy idle cut (R8). */
  closeStreams(reason: StreamCloseReason = { kind: 'disconnected' }, streamId?: number): void {
    for (const [id, s] of [...this.streams]) {
      if (streamId !== undefined && streamId !== id) continue
      this.streams.delete(id)
      s.socket.send({ type: 'StreamClosed', data: { stream_id: id, reason } })
    }
  }

  /** Ids of the streams currently open. */
  openStreams(): number[] {
    return [...this.streams.keys()]
  }

  // --- HTTP ---------------------------------------------------------------------------------

  private record(req: Request): void {
    const url = new URL(req.url())
    this.http.push({ method: req.method(), path: url.pathname, csrf: req.headers()['x-csrf-token'] ?? null })
  }

  private meResponse(req: Request) {
    this.record(req)
    if (this.me === null) {
      return { status: 401, json: { error: 'unauthenticated', providers: this.providers } }
    }
    return { status: 200, json: this.me }
  }

  private exportResponse(req: Request) {
    this.record(req)
    if (this.me === null) return { status: 401, json: { error: 'unauthenticated', providers: this.providers } }
    if (this.me.role !== 'admin') return { status: 403, json: { error: 'forbidden' } }
    if (!this.me.network.trusted) return this.refuse('AuditExport')
    const q = new URL(req.url()).searchParams
    const f: AuditFilter = {}
    for (const k of ['stack', 'actor', 'action', 'from', 'to'] as const) {
      const v = q.get(k)
      if (v !== null) (f as Record<string, string>)[k] = v
    }
    const body = this.auditMatching(f).map((e) => JSON.stringify(e) + '\n').join('')
    return {
      status: 200,
      contentType: 'application/x-ndjson',
      headers: { 'content-disposition': 'attachment; filename="depths-audit.jsonl"' },
      body,
    }
  }

  /** Spec 052: what an untrusted network may send (FR-009). Confirm goes by its preview's kind. */
  private restrictedOk(cmd: Cmd): boolean {
    switch (cmd.type) {
      case 'PreviewOp':
        return RESTRICTED_OPS.has(cmd.data.op.kind)
      case 'Confirm': {
        // 053 R12: listing is all the restricted view gets of files, so no file change confirms.
        if (this.files.owns(cmd.data.preview_id)) return false
        const p = this.previews.get(cmd.data.preview_id)
        return p === undefined || RESTRICTED_OPS.has(p.preview.kind)
      }
      case 'ReadFile':
      case 'FolderSize':
      case 'PreviewFileOp':
      case 'ListHistory':
      case 'ReadVersion':
      case 'PreviewSave':
      case 'RevealEnv':
      case 'OpenShell':
      case 'ShellInput':
      case 'ShellResize':
      case 'GetAudit':
      case 'PutUser':
      case 'RemoveUser':
        return false
      default:
        return true
    }
  }

  /** The 403 body of a route refused for its network, audited. */
  private refuse(what: string) {
    const d = (this.me ?? fixtures.me()).network
    this.recordAudit({
      action: 'network_refused',
      outcome: { kind: 'denied', reason: 'untrusted_network' },
      network: { network: d.network, cmd: what },
    })
    return {
      status: 403,
      json: { error: 'untrusted_network', network: d.network, reason: d.reason, ...(d.matures_at ? { matures_at: d.matures_at } : {}) },
    }
  }

  /** 053: the two file routes. Gated as the socket is: session, network, then `files`' own gates. */
  private filesResponse(req: Request) {
    this.record(req)
    const me = this.me
    if (me === null) return { status: 401, json: { error: 'unauthenticated', providers: this.providers } }
    const url = new URL(req.url())
    const path = decodeURIComponent(url.pathname)
    const m = /^\/api\/v1\/stacks\/([^/]+)\/files\/(download|upload)$/.exec(path)
    const stack = m?.[1] ?? ''
    const route = m?.[2]
    const method = req.method()
    if (route === undefined || (route === 'download') !== (method === 'GET') || (route === 'upload') !== (method === 'POST')) {
      return { status: 404, json: { error: 'not_found' } }
    }
    if (route === 'upload' && req.headers()['x-csrf-token'] !== me.csrf_token) return { status: 403, json: { error: 'csrf' } }
    if (!me.network.trusted) {
      this.recordAudit({
        action: 'network_refused',
        stack,
        outcome: { kind: 'denied', reason: 'untrusted_network' },
        network: { network: me.network.network, cmd: `${method} ${path}` },
      })
      return { status: 403, json: { kind: 'untrusted_network' } }
    }
    const may = (s: string): boolean => this.mayStack(me, s)
    const reply: FileHttpReply =
      route === 'download'
        ? this.files.download(stack, url.searchParams, may)
        : this.files.upload(stack, url.searchParams, req.postDataBuffer() ?? new Uint8Array(0), may)
    return {
      status: reply.status,
      ...(reply.headers ? { headers: reply.headers } : {}),
      ...(reply.contentType ? { contentType: reply.contentType } : {}),
      ...(reply.body ? { body: Buffer.from(reply.body) } : { json: reply.json }),
    }
  }

  /** Whether `me` may work on `stack`: an admin, or a user it is assigned to. */
  private mayStack(me: Me, stack: string): boolean {
    return me.role === 'admin' || me.stacks.includes(stack)
  }

  /** The decision for `n` after an answer: never trusted by the answer alone (FR-006). */
  private decisionAfter(n: NetworkView): TrustDecision {
    if (n.trusted) return { trusted: true, reason: n.reason, network: n.key }
    return n.answer === 'home'
      ? { trusted: false, reason: 'maturing', matures_at: this.maturesAt, network: n.key }
      : { trusted: false, reason: n.answer === 'public' ? 'public' : 'unknown', network: n.key }
  }

  /** Spec 052's routes: own networks and sessions from any network; admin whitelist from a trusted one. */
  private networkResponse(req: Request) {
    this.record(req)
    const me = this.me
    if (me === null) return { status: 401, json: { error: 'unauthenticated', providers: this.providers } }
    const method = req.method()
    const path = decodeURIComponent(new URL(req.url()).pathname)
    const json = (status: number, body: unknown) => ({ status, json: body })
    if (method !== 'GET' && req.headers()['x-csrf-token'] !== me.csrf_token) return json(403, { error: 'csrf' })
    const audit = (action: AuditEntry['action'], network: AuditEntry['network']): void => {
      this.recordAudit({ action, outcome: { kind: 'succeeded' }, ...(network ? { network } : {}) })
    }

    if (path === '/api/v1/networks' && method === 'GET') return json(200, this.networks)
    if (path === '/api/v1/networks/answer' && method === 'POST') {
      const body = req.postDataJSON() as AnswerNetwork
      if (body.answer !== 'home' && body.answer !== 'public') return json(400, { error: 'invalid_answer' })
      let n = this.networks.find((x) => x.key === body.network)
      if (!n && body.network === me.network.network) {
        n = fixtures.networkView(body.network, { current: true })
        this.networks.push(n)
      }
      if (!n) return json(404, { error: 'unknown_network' })
      n.answer = body.answer
      if (n.source === 'seen') n.source = 'answer'
      const d = this.decisionAfter(n)
      n.reason = d.reason
      if (body.network === me.network.network) this.me = { ...me, network: d, needs_answer: false }
      audit('network_answer', { network: n.key, answer: body.answer })
      return json(200, d)
    }

    if (path === '/api/v1/sessions' && method === 'GET') return json(200, this.sessionList)
    if (path === '/api/v1/sessions/end-others' && method === 'POST') {
      const ended = this.sessionList.filter((x) => !x.current).length
      this.sessionList = this.sessionList.filter((x) => x.current)
      audit('session_end', { session: 'others' })
      return json(200, { ended })
    }
    const one = /^\/api\/v1\/sessions\/([^/]+)$/.exec(path)
    if (one && method === 'DELETE') {
      const hint = one[1] ?? ''
      if (!this.sessionList.some((x) => x.id_hint === hint)) return json(404, { error: 'unknown_session' })
      this.sessionList = this.sessionList.filter((x) => x.id_hint !== hint)
      audit('session_end', { session: hint })
      return json(200, { ended: 1 })
    }

    const u = /^\/api\/v1\/users\/([^/]+)\/(sessions|networks)(?:\/(.+))?$/.exec(path)
    if (!u) return json(404, { error: 'not_found' })
    if (me.role !== 'admin') return json(403, { error: 'forbidden' })
    const id = u[1] ?? ''
    const rest = u[3]
    if (u[2] === 'sessions' && rest === undefined) {
      if (method === 'GET') return json(200, this.userSessions[id] ?? [])
      if (method === 'DELETE') {
        const ended = (this.userSessions[id] ?? []).length
        this.userSessions[id] = []
        audit('session_end', { person: id, session: 'all' })
        return json(200, { ended })
      }
    }
    if (u[2] === 'networks') {
      const list = this.userNetworks[id] ?? []
      if (method === 'GET' && rest === undefined) return json(200, list)
      // Whitelisting, removing and forgetting: from a trusted network only (FR-006, FR-017).
      if (!me.network.trusted) return this.refuse(`${method} ${path}`)
      if (method === 'POST' && rest === 'whitelist') {
        const body = req.postDataJSON() as WhitelistNetwork
        if (!/^[0-9a-f:.]+(\/\d{1,3})?$/i.test(body.cidr)) return json(400, { error: 'invalid_network' })
        const key = body.cidr.includes('/') ? body.cidr : `${body.cidr}/${body.cidr.includes(':') ? '128' : '32'}`
        const entry = list.find((x) => x.key === key) ?? fixtures.networkView(key, { days: [] })
        Object.assign(entry, {
          source: 'whitelist',
          trusted: true,
          reason: 'whitelist',
          whitelisted_by: `${me.identity.provider}:${me.identity.subject}`,
          at: NOW,
          ...(body.label ? { label: body.label } : {}),
        })
        this.userNetworks[id] = list.includes(entry) ? list : [...list, entry]
        audit('network_whitelist_add', { network: key, person: id })
        return json(200, this.userNetworks[id])
      }
      if (method === 'DELETE' && rest !== undefined) {
        const entry = list.find((x) => x.key === rest)
        if (!entry) return json(404, { error: 'unknown_network' })
        if (entry.source === 'whitelist') {
          // Removing a whitelist entry leaves the network known, and unknown again.
          const kept: NetworkView = { ...entry, source: 'seen', trusted: false, reason: 'unknown' }
          delete kept.whitelisted_by
          delete kept.at
          delete kept.label
          this.userNetworks[id] = list.map((x) => (x === entry ? kept : x))
          audit('network_whitelist_remove', { network: rest, person: id })
        } else {
          this.userNetworks[id] = list.filter((x) => x !== entry)
          audit('network_forget', { network: rest, person: id })
        }
        return json(200, this.userNetworks[id])
      }
    }
    return json(404, { error: 'not_found' })
  }

  /** `audit` narrowed by a filter, as `GetAudit` and the export both read it. */
  auditMatching(f: AuditFilter): AuditEntry[] {
    return this.audit.filter(
      (e) =>
        (f.stack === undefined || e.stack === f.stack) &&
        (f.actor === undefined || `${e.actor.provider}:${e.actor.subject}` === f.actor) &&
        (f.action === undefined || e.action === f.action) &&
        (f.from === undefined || e.at >= f.from) &&
        (f.to === undefined || e.at <= f.to),
    )
  }

  /** Appends an entry for the signed-in person, as the server records each attempt (FR-035). */
  private recordAudit(over: Partial<AuditEntry> & Pick<AuditEntry, 'action' | 'outcome'>): void {
    const who = this.me?.identity ?? fixtures.me().identity
    this.audit.push({
      at: NOW,
      actor: { provider: who.provider, subject: who.subject, username: who.username },
      stack: null,
      preview_id: null,
      operation_id: null,
      argv: [],
      target: null,
      diff_summary: null,
      source_ip: '127.0.0.1',
      ...over,
    })
  }

  private auth(req: Request) {
    this.record(req)
    const path = new URL(req.url()).pathname
    const csrfOk = this.me !== null && req.headers()['x-csrf-token'] === this.me.csrf_token
    if (path === '/auth/logout' && req.method() === 'POST') {
      if (csrfOk) this.me = null
      return Promise.resolve(csrfOk ? { status: 200, json: {} } : { status: 403, json: { error: 'csrf' } })
    }
    if (path === '/auth/refresh' && req.method() === 'POST') {
      return Promise.resolve(csrfOk ? { status: 204, body: '' } : { status: 401, json: { error: 'unauthenticated' } })
    }
    // `/auth/login` and callbacks: a stub page, so a click on a provider link lands somewhere.
    return Promise.resolve({ status: 200, contentType: 'text/plain', body: `fake ${path}` })
  }

  // --- WebSocket ----------------------------------------------------------------------------

  private open(): FakeSocket[] {
    return this.sockets.filter((s) => s.closed === null)
  }

  private accept(route: WebSocketRoute): void {
    const socket = new FakeSocket(route)
    this.sockets.push(socket)
    route.onClose(() => {
      socket.closed ??= 1000
    })
    route.onMessage((raw) => {
      let frame: unknown
      try {
        frame = JSON.parse(typeof raw === 'string' ? raw : raw.toString('utf8'))
      } catch {
        socket.close(1007, 'malformed frame')
        return
      }
      if (!socket.ready) this.hello(socket, frame)
      else this.command(socket, frame)
    })
  }

  private hello(socket: FakeSocket, frame: unknown): void {
    socket.hellos.push(frame)
    const h = frame as Partial<Hello> | null
    if (this.me === null) {
      socket.close(CloseCode.Unauthenticated, 'unauthenticated')
    } else if (h?.protocol_version !== PROTOCOL_VERSION || h.csrf_token !== this.me.csrf_token) {
      socket.close(CloseCode.Forbidden, 'bad hello')
    } else if (this.helloClose !== null) {
      socket.close(this.helloClose)
    } else {
      socket.ready = true
    }
  }

  private command(socket: FakeSocket, frame: unknown): void {
    const env = frame as Partial<CmdEnvelope> | null
    if (typeof env?.id !== 'number' || typeof env.cmd?.type !== 'string') {
      socket.send({ type: 'Invalid', data: { reason: 'expected a CmdEnvelope' } })
      return
    }
    const { id, cmd } = env as CmdEnvelope
    socket.received.push(env as CmdEnvelope)
    const w = this.waiters.findIndex((x) => x.type === cmd.type)
    if (w >= 0) {
      const [waiter] = this.waiters.splice(w, 1)
      this.awaited.add(cmd)
      waiter?.resolve(cmd)
    }

    // The session as it is now: signed out or demoted since the socket opened ends it (FR-033).
    if (this.me === null) {
      socket.close(CloseCode.Unauthenticated)
      return
    }
    if (this.me.role === 'pending' || (this.me.role === 'user' && this.me.stacks.length === 0)) {
      socket.close(CloseCode.Forbidden)
      return
    }

    // Spec 052: the network gate, after the session checks and before any handler.
    if (!this.me.network.trusted && !this.restrictedOk(cmd)) {
      this.recordAudit({
        action: 'network_refused',
        stack: stackOf(cmd),
        outcome: { kind: 'denied', reason: 'untrusted_network' },
        network: { network: this.me.network.network, cmd: cmd.type },
      })
      socket.send({ type: 'Denied', data: { reason: 'untrusted_network' } }, id)
      return
    }

    const ctx: HandlerContext = { backend: this, socket, id }
    const scripted = this.handlers.get(cmd.type)?.(cmd, ctx)
    const events = scripted ?? this.respond(cmd, socket, id)
    // Every refusal is audited, with the stack it named (DenyReason: "Audited.").
    for (const e of events) {
      if (e.type === 'Denied') this.recordAudit({ action: 'denied', stack: stackOf(cmd), outcome: { kind: 'denied', reason: e.data.reason } })
    }
    for (const e of events) socket.send(e, id)
  }

  /** The default reply to each command. */
  private respond(cmd: Cmd, socket: FakeSocket, id: number): Event[] {
    const me = this.me ?? fixtures.me()
    const admin = me.role === 'admin'
    const may = (stack: string): boolean => this.mayStack(me, stack)
    const denied = (reason: DenyReason = 'not_permitted'): Event[] => [{ type: 'Denied', data: { reason } }]
    const find = (name: string): Stack | undefined => this.stacks.stacks.find((s) => s.name === name)

    switch (cmd.type) {
      case 'ListStacks':
        return [{ type: 'Stacks', data: this.visibleStacks(me) }]

      case 'GetStack': {
        const s = find(cmd.data.name)
        if (!s || !may(s.name)) return denied()
        const detail = this.details[s.name] ?? fixtures.detail(s)
        // As the server: the compose file and `.env` go to a trusted network only (no refusal).
        if (!me.network.trusted) return [{ type: 'StackDetail', data: { ...detail, compose_text: null, env: null, compose_warnings: [] } }]
        return [{ type: 'StackDetail', data: detail }]
      }

      case 'RevealEnv': {
        // Admin-only, as the server's role table has it (FR-021).
        if (!admin || !may(cmd.data.name)) return denied()
        return [{ type: 'EnvRevealed', data: { name: cmd.data.name, env_text: this.env[cmd.data.name] ?? '' } }]
      }

      case 'PreviewOp': {
        const { stack, op } = cmd.data
        const s = find(stack)
        if (!s || !may(stack) || (adminOnly(op) && !admin)) return denied()
        if (!s.valid_name) return [{ type: 'Invalid', data: { reason: 'not a valid compose project name' } }]
        const busy = this.busy(stack)
        if (busy) return busy
        const findings = ['up', 'update', 'service_up'].includes(op.kind) ? (this.lint[stack] ?? []) : []
        const blocked = this.blocked(findings, admin)
        if (blocked) return blocked
        return [{ type: 'Preview', data: this.withFindings(this.makePreview(s, op.kind, stepsFor(stack, op), null, isDestructive(op)), findings) }]
      }

      case 'PreviewSave': {
        const { draft, deploy } = cmd.data
        const s = find(draft.stack)
        const creating = draft.base_hash === undefined
        if (creating ? !admin : !s || !may(draft.stack)) return denied()
        if (!me.features.edit) return denied('feature_disabled')
        if (creating && s) return [{ type: 'Invalid', data: { reason: `a stack named ${draft.stack} already exists` } }]
        const errors = this.validationErrors[draft.stack]
        if (errors?.length) {
          return [{ type: 'ValidationFailed', data: { errors: errors.map((e) => ({ file: 'compose' as const, ...e })) } }]
        }
        const current = s ? (this.details[s.name] ?? fixtures.detail(s)) : null
        if (current && draft.base_hash !== current.content_hash) {
          return [
            {
              type: 'Conflict',
              data: { current_text: current.compose_text ?? '', current_env: current.env, current_hash: current.content_hash },
            },
          ]
        }
        const target = s ?? fixtures.stack(draft.stack, { status: 'not_deployed' })
        const cwd = `/srv/stacks/${draft.stack}`
        const steps: Step[] = [
          { argv: ['docker', 'compose', '-f', '-', 'config', '--quiet'], cwd, stdin: 'draft' },
          ...(deploy ? stepsFor(draft.stack, { kind: 'up' }) : []),
        ]
        const diff = naiveDiff('compose.yaml', current?.compose_text ?? '', draft.compose)
        const findings = this.lint[draft.stack] ?? fakeLint(draft.compose)
        const blocked = this.blocked(findings, admin)
        if (blocked) return blocked
        return [{ type: 'Preview', data: this.withFindings(this.makePreview(target, creating ? 'create' : 'save', steps, diff, false), findings) }]
      }

      case 'Confirm': {
        // 051 FR-018: a preview with escape findings under `enforce` runs only acknowledged.
        if (this.files.owns(cmd.data.preview_id)) return this.files.confirm(cmd.data.preview_id, may)
        const held = this.previews.get(cmd.data.preview_id)
        if (held?.preview.ack_required && !held.used && cmd.data.acknowledge_findings !== true) {
          return [{ type: 'Denied', data: { reason: 'compose_blocked', detail: 'acknowledge the findings to deploy' } }]
        }
        return this.confirm(cmd.data.preview_id, socket, id)
      }

      case 'LintCompose': {
        const { stack, compose_text } = cmd.data
        if (!may(stack)) return denied()
        return [{ type: 'LintResult', data: { stack, findings: this.lint[stack] ?? fakeLint(compose_text) } }]
      }

      case 'Cancel': {
        const i = this.held.findIndex((h) => h.operationId === cmd.data.operation_id)
        if (i < 0) return [{ type: 'Invalid', data: { reason: 'no such operation' } }]
        this.held.splice(i, 1)
        return [{ type: 'OpFinished', data: { operation_id: cmd.data.operation_id, outcome: { kind: 'cancelled' } } }]
      }

      case 'OpenLogs': {
        const t = cmd.data.target
        const key = t.kind === 'container' ? t.container : t.service ? `${t.stack}/${t.service}` : t.stack
        if (t.kind === 'stack' ? !may(t.stack) : !admin && !this.containerIn(t.container, me)) return denied()
        const streamId = this.openStream(socket, 'logs')
        const lines = (this.logs[key] ?? []).slice(-cmd.data.tail)
        return [
          { type: 'StreamOpened', data: { stream_id: streamId } },
          ...(lines.length ? [{ type: 'LogChunk' as const, data: { stream_id: streamId, lines } }] : []),
        ]
      }

      case 'OpenShell': {
        if (!me.features.shell) return denied('feature_disabled')
        if (!admin && !this.containerIn(cmd.data.container, me)) return denied()
        // 051 FR-008: the container's rating, as the server's fresh inspect would find it.
        const rule = this.securityOf(cmd.data.container)?.shell
        if (rule === 'refused') return [{ type: 'Denied', data: { reason: 'shell_escape', detail: 'it can reach the host' } }]
        if (rule === 'admin_only' && !admin) {
          return [{ type: 'Denied', data: { reason: 'shell_root_admin_only', detail: 'it runs as root on the host' } }]
        }
        const streamId = this.openStream(socket, 'shell')
        return [
          { type: 'StreamOpened', data: { stream_id: streamId } },
          { type: 'ShellOutput', data: { stream_id: streamId, bytes_b64: Buffer.from(this.shellBanner).toString('base64') } },
        ]
      }

      case 'ShellInput': {
        // A shell that echoes its input.
        const s = this.streams.get(cmd.data.stream_id)
        if (s?.socket !== socket || s.kind !== 'shell') return []
        return [{ type: 'ShellOutput', data: { stream_id: cmd.data.stream_id, bytes_b64: cmd.data.bytes_b64 } }]
      }

      case 'ShellResize':
      case 'UnwatchStats':
      case 'UnwatchHost':
        return []

      case 'WatchHost':
        // As the server: the permission first, so a user never learns whether it is on.
        if (!admin) return denied()
        if (!me.features.host_metrics) return denied('feature_disabled')
        // A live sample, not a reply: the server's ticks carry no `reply_to`.
        queueMicrotask(() => socket.send({ type: 'HostSample', data: this.host }))
        return []

      case 'CloseStream': {
        const s = this.streams.get(cmd.data.stream_id)
        if (s?.socket !== socket) return []
        this.streams.delete(cmd.data.stream_id)
        return [{ type: 'StreamClosed', data: { stream_id: cmd.data.stream_id, reason: { kind: 'closed' } } }]
      }

      case 'WatchStats': {
        const ids = cmd.data.targets.flatMap((t) =>
          // Samples carry the full id, as docker reports it, whatever ref the target named.
          t.kind === 'container'
            ? [this.stacks.stacks.flatMap((s) => this.containersOf(s.name)).find((c) => c.name === t.container)?.id ?? t.container]
            : may(t.stack)
              ? this.containersOf(t.stack).map((c) => c.id)
              : [],
        )
        const samples = this.samples.length ? this.samples : ids.map((cid) => fixtures.sample(cid))
        return [{ type: 'StatsSample', data: { samples } }]
      }

      case 'GetAudit': {
        if (!admin) return denied()
        // Newest first, `auditPageSize` at a time; the cursor is the offset, opaque to the UI.
        const all = this.auditMatching(cmd.data.filter).slice().reverse()
        const start = cmd.data.cursor === undefined ? 0 : Number(cmd.data.cursor)
        const entries = all.slice(start, start + this.auditPageSize)
        const end = start + entries.length
        return [{ type: 'AuditPage', data: { entries, next_cursor: end < all.length ? String(end) : null } }]
      }

      case 'GetUsers':
        return admin ? [{ type: 'Users', data: this.users }] : denied()

      case 'PutUser': {
        if (!admin) return denied()
        if (cmd.data.role === 'pending') return denied('role_not_assignable')
        const all = [...this.users.users, ...this.users.pending]
        const existing = all.find((u) => u.id === cmd.data.id)
        if (existing?.owner) return denied('owner')
        const next: User = {
          ...(existing ?? fixtures.user(cmd.data.id, { first_seen: null, last_seen: null, username: null })),
          role: cmd.data.role,
          stacks: cmd.data.stacks,
        }
        this.setUsers({
          users: [...this.users.users.filter((u) => u.id !== next.id), next],
          pending: this.users.pending.filter((u) => u.id !== next.id),
        }, socket)
        const before = existing && existing.role !== 'pending' ? grantOf(existing) : null
        this.recordAudit({
          action: before ? 'user_changed' : 'user_added',
          target: { id: next.id, before, after: grantOf(next) },
          outcome: { kind: 'succeeded' },
        })
        return [{ type: 'Users', data: this.users }]
      }

      // 053: the volume explorer, answered from `files`.
      case 'ListMounts':
      case 'ListDir':
      case 'ReadFile':
      case 'FolderSize':
      case 'PreviewFileOp':
      case 'ListHistory':
      case 'ReadVersion':
        return this.files.handle(cmd, may)

      case 'RemoveUser': {
        if (!admin) return denied()
        const u = [...this.users.users, ...this.users.pending].find((x) => x.id === cmd.data.id)
        if (u?.owner) return denied('owner')
        this.setUsers({
          users: this.users.users.filter((x) => x.id !== cmd.data.id),
          pending: this.users.pending.filter((x) => x.id !== cmd.data.id),
        }, socket)
        this.recordAudit({
          action: 'user_removed',
          target: { id: cmd.data.id, before: u ? grantOf(u) : null, after: null },
          outcome: { kind: 'succeeded' },
        })
        return [{ type: 'Users', data: this.users }]
      }

      // Spec 054. No UI drives these yet: the fake answers as a server with no passkeys and no
      // guard exceptions, and refuses every ceremony.
      case 'GetVerification':
        return [{
          type: 'Verification',
          data: { verified: false, at: null, window_secs: 300, enrolled: false, required: false, recovering: false },
        }]

      case 'ListPasskeys':
        return [{ type: 'Passkeys', data: { items: [] } }]

      case 'GetGuardExceptions':
        if (!may(cmd.data.stack)) return denied()
        return [{ type: 'GuardExceptions', data: { stack: cmd.data.stack, items: [] } }]

      case 'PasskeyBegin':
      case 'PasskeyFinish':
      case 'PreviewPasskeyRemove':
        return [{ type: 'PasskeyRefused', data: { reason: 'invalid' } }]

      case 'RecoverPasskeys':
        return [{ type: 'PasskeyRefused', data: { reason: 'recovery_invalid' } }]
    }
  }

  /** What a session may see: a `user` gets their stacks only, and no foreign or standalone group. */
  visibleStacks(me: Me): StackList {
    if (me.role === 'admin') return this.stacks
    return { ...this.stacks, stacks: this.stacks.stacks.filter((s) => me.stacks.includes(s.name)), foreign: [], standalone: [] }
  }

  private setUsers(list: UserList, origin: FakeSocket): void {
    this.users = list
    for (const s of this.open()) if (s !== origin) s.send({ type: 'UsersChanged', data: list })
  }

  private containersOf(stack: string): Container[] {
    return this.stacks.stacks.find((s) => s.name === stack)?.services.flatMap((v) => v.containers) ?? []
  }

  /** 051: a container's rating, from the stacks' fixtures. */
  private securityOf(ref: string): ContainerSecurity | undefined {
    return this.stacks.stacks
      .flatMap((st) => st.services.flatMap((v) => v.containers))
      .find((c) => c.id === ref || c.name === ref)?.security
  }
  /** 051 FR-018: escape findings under `enforce` refuse a user outright. */
  private blocked(findings: ComposeWarning[], admin: boolean): Event[] | null {
    if (this.composePolicy !== 'enforce' || admin) return null
    const escapes = findings.filter((w) => w.severity === 'escape')
    if (escapes.length === 0) return null
    const detail = escapes.map((w) => `${w.service}: ${w.message}`).join('; ')
    return [{ type: 'Denied', data: { reason: 'compose_blocked', detail } }]
  }
  /** 051 FR-016: a preview carries its findings; escape ones under `enforce` need an acknowledgement. */
  private withFindings(preview: Preview, findings: ComposeWarning[]): Preview {
    preview.warnings = findings
    preview.ack_required = this.composePolicy === 'enforce' && findings.some((w) => w.severity === 'escape')
    return preview
  }
  private containerIn(ref: string, me: Me): boolean {
    return me.stacks.some((st) => this.containersOf(st).some((c) => c.id === ref || c.name === ref))
  }

  private busy(stack: string): Event[] | null {
    const h = this.held.find((x) => x.stack === stack)
    if (!h) return null
    const who = this.me?.identity ?? fixtures.me().identity
    return [
      {
        type: 'Busy',
        data: { stack, operation_id: h.operationId, kind: h.kind, actor: { provider: who.provider, subject: who.subject, username: who.username } },
      },
    ]
  }

  private makePreview(s: Stack, kind: OperationKind, steps: Step[], diff: string | null, destructive: boolean): Preview {
    const preview: Preview = {
      preview_id: `pv-${String(++this.seq)}`,
      stack: s.name,
      kind,
      steps,
      file_diff: diff,
      reaches: {
        containers: s.services.flatMap((v) => v.containers.map((c) => c.name)),
        networks: [`${s.name}_default`],
        volumes: [],
      },
      destructive,
      stack_hash: s.content_hash,
      expires_at: '2026-10-23T12:05:00Z',
    }
    this.previews.set(preview.preview_id, { preview, used: false, expired: false })
    return preview
  }

  private confirm(previewId: string, socket: FakeSocket, replyTo: number): Event[] {
    const p = this.previews.get(previewId)
    const voidWith = (reason: PreviewVoidReason): Event[] => [{ type: 'PreviewVoid', data: { reason } }]
    if (!p) return voidWith('unknown')
    if (p.used) return voidWith('used')
    if (p.expired) return voidWith('expired')
    const current = this.stacks.stacks.find((s) => s.name === p.preview.stack)
    if (current && current.content_hash !== p.preview.stack_hash) return voidWith('stack_changed')
    const busy = this.busy(p.preview.stack)
    if (busy) return busy
    p.used = true

    const operationId = `op-${String(++this.seq)}`
    const events: Event[] = [
      { type: 'OpStarted', data: { operation_id: operationId } },
      ...this.op.output.map((chunk): Event => ({ type: 'OpOutput', data: { operation_id: operationId, chunk } })),
    ]
    if (this.op.hold) this.held.push({ operationId, stack: p.preview.stack, kind: p.preview.kind, socket, replyTo })
    else events.push({ type: 'OpFinished', data: { operation_id: operationId, outcome: this.op.outcome } })
    return events
  }

  private openStream(socket: FakeSocket, kind: OpenStream['kind']): number {
    const id = ++this.seq
    this.streams.set(id, { socket, kind })
    return id
  }
}

/** Grants helpers for stories that switch who is signed in. */
export const sessions = {
  admin: (over: Partial<Me> = {}): Me => fixtures.me({ role: 'admin', ...over }),
  user: (stacks: string[], over: Partial<Me> = {}): Me => fixtures.me({ role: 'user' as Role, stacks, ...over }),
  pending: (over: Partial<Me> = {}): Me => fixtures.me({ role: 'pending', stacks: [], ...over }),
  /** An admin on a network Depths has not seen for them (spec 052): asked home or public. */
  untrusted: (over: Partial<Me> = {}): Me =>
    fixtures.me({ network: { trusted: false, reason: 'unknown', network: '198.51.100.7/32' }, needs_answer: true, ...over }),
}

/**
 * `test` with a `backend` fixture already installed on `page` (an admin session, no stacks).
 * Script it before `page.goto`.
 */
export const test = base.extend<{ backend: FakeBackend }>({
  // `provide` rather than Playwright's usual `use`, which the React hooks lint rule reads as a hook.
  backend: async ({ page }, provide) => {
    await provide(await FakeBackend.install(page))
  },
})
