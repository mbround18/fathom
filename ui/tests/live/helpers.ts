/**
 * Live e2e helpers: the `app` fixture (the real UI wired to the devbridge the global setup
 * started), a fenced `kubectl`, polling, and sandbox baseline management.
 *
 * Safety: `kubectl()` always runs with `--context $FATHOM_LIVE_CONTEXT -n fathom-dev` and throws
 * before running anything that names another namespace, another cluster, or a kind outside the
 * namespaced allow-list. Every object a spec creates carries `fathom.dev/e2e=true`.
 */
import { execFileSync } from 'node:child_process'

import {
  test as base,
  expect,
  type Locator,
  type Page,
  type WebSocketRoute,
} from '@playwright/test'

import type { BackendCmd, DevBridgeClientMsg, DevBridgeServerMsg } from '../../src/protocol/types'
import type {} from '../../src/protocol/ws-transport'
import { listPane, navigateTo, row, table } from '../e2e/helpers'

import { APP_ORIGIN, E2E_LABEL, ENV, LIVE_NAMESPACE, liveContext } from './env'

export {
  DEPLOYMENTS,
  inspector,
  isPhone,
  listPane,
  navigateTo,
  PODS,
  row,
  table,
} from '../e2e/helpers'
export { E2E_LABEL, LIVE_NAMESPACE } from './env'

// ---------------------------------------------------------------------------- kubectl

/** Kinds (any spelling kubectl accepts, lower-cased, group suffix stripped) that are namespaced. */
const NAMESPACED_KINDS = new Set([
  'all',
  ...['pod', 'pods', 'po'],
  ...['deployment', 'deployments', 'deploy'],
  ...['replicaset', 'replicasets', 'rs'],
  ...['statefulset', 'statefulsets', 'sts'],
  ...['daemonset', 'daemonsets', 'ds'],
  ...['job', 'jobs'],
  ...['cronjob', 'cronjobs', 'cj'],
  ...['service', 'services', 'svc'],
  ...['configmap', 'configmaps', 'cm'],
  ...['secret', 'secrets'],
  ...['event', 'events', 'ev'],
  ...['endpoints', 'ep', 'endpointslice', 'endpointslices'],
  ...['persistentvolumeclaim', 'persistentvolumeclaims', 'pvc'],
  ...['serviceaccount', 'serviceaccounts', 'sa'],
  ...['role', 'roles', 'rolebinding', 'rolebindings'],
  ...['poddisruptionbudget', 'poddisruptionbudgets', 'pdb'],
  ...['horizontalpodautoscaler', 'horizontalpodautoscalers', 'hpa'],
  ...['networkpolicy', 'networkpolicies', 'netpol'],
  ...['resourcequota', 'resourcequotas', 'quota'],
  ...['limitrange', 'limitranges', 'limits'],
  ...['ingress', 'ingresses', 'ing'],
  ...['lease', 'leases'],
])

/** Verbs the live suite may use. `config`, `proxy`, `auth`, `exec`, … are not among them. */
const VERBS = new Set([
  'get',
  'describe',
  'logs',
  'wait',
  'create',
  'apply',
  'delete',
  'scale',
  'rollout',
  'label',
  'annotate',
])

/** Flags that would move the command to another namespace, cluster, identity, or input. */
const FORBIDDEN_FLAGS =
  /^(-n|--namespace|-A|--all-namespaces|--context|--kubeconfig|--cluster|--server|-s|--user|--token|--as|--as-group|--as-uid|--certificate-authority|--client-certificate|--client-key|--insecure-skip-tls-verify|-f|--filename|-k|--kustomize|-R|--recursive|--raw)(=.*)?$/

/** Flags whose value follows as a separate argument (so it is not mistaken for a resource). */
const VALUE_FLAGS = new Set([
  '-o',
  '--output',
  '-l',
  '--selector',
  '-c',
  '--container',
  '--field-selector',
  '--for',
  '--timeout',
  '--replicas',
  '--sort-by',
  '--tail',
  '--since',
  '-p',
  '--patch',
  '--type',
])

function kindOf(token: string): string {
  return (token.split('/')[0] ?? '').split('.')[0]?.toLowerCase() ?? ''
}

function refuse(args: readonly string[], why: string): never {
  throw new Error(`[live e2e] refusing 'kubectl ${args.join(' ')}': ${why}`)
}

/** A Kubernetes object passed to `create`/`apply` on stdin. */
export interface KubeObject {
  apiVersion: string
  kind: string
  metadata?: { name?: string; namespace?: string; labels?: Record<string, string> }
  items?: KubeObject[]
  [key: string]: unknown
}

function checkObject(args: readonly string[], obj: KubeObject): void {
  if (obj.kind === 'List') {
    for (const item of obj.items ?? []) checkObject(args, item)
    return
  }
  if (!NAMESPACED_KINDS.has(obj.kind.toLowerCase())) {
    refuse(args, `kind ${obj.kind} is cluster-scoped or not on the namespaced allow-list`)
  }
  const ns = obj.metadata?.namespace
  if (ns !== undefined && ns !== LIVE_NAMESPACE) {
    refuse(args, `object ${obj.kind}/${obj.metadata?.name ?? '?'} targets namespace '${ns}'`)
  }
}

/** Throws unless `args` stays inside `fathom-dev` (see the module doc). Exported for tests. */
export function checkKubectlArgs(args: readonly string[], input?: KubeObject): void {
  const [verb, ...rest] = args
  if (verb === undefined || !VERBS.has(verb)) {
    refuse(args, `verb '${verb ?? ''}' is not allowed (allowed: ${[...VERBS].join(', ')})`)
  }
  const positional: string[] = []
  for (let i = 0; i < rest.length; i++) {
    const arg = rest[i] ?? ''
    if (arg === '-f' && rest[i + 1] === '-' && (verb === 'create' || verb === 'apply') && input) {
      i++
      continue
    }
    if (FORBIDDEN_FLAGS.test(arg)) {
      refuse(args, `flag '${arg}' could leave namespace '${LIVE_NAMESPACE}' / the live context`)
    }
    if (arg.startsWith('-')) {
      if (VALUE_FLAGS.has(arg)) i++
      continue
    }
    positional.push(arg)
  }
  if (verb === 'create' || verb === 'apply') {
    if (!input) refuse(args, `'${verb}' is only allowed as '-f -' with a checked object`)
    checkObject(args, input)
    if (positional.length > 0) refuse(args, `'${verb}' takes no resource arguments here`)
    return
  }
  // `rollout <subcommand> <resource>`
  const resources = verb === 'rollout' ? positional.slice(1) : positional
  // `logs <pod>` names a pod, not a kind; the kind-less form is fine.
  const kindTokens =
    verb === 'logs'
      ? resources.filter((r) => r.includes('/'))
      : [
          ...(resources[0] !== undefined ? resources[0].split(',') : []),
          ...resources.slice(1).filter((r) => r.includes('/')),
        ]
  for (const token of kindTokens) {
    const kind = kindOf(token)
    if (!NAMESPACED_KINDS.has(kind)) {
      refuse(args, `kind '${kind}' is cluster-scoped or not on the namespaced allow-list`)
    }
  }
}

/**
 * Run kubectl in the live sandbox: ALWAYS `--context $FATHOM_LIVE_CONTEXT -n fathom-dev`.
 * `input` (for `create -f -` / `apply -f -`) is checked object by object before it is sent.
 */
export function kubectl(args: readonly string[], opts: { input?: KubeObject } = {}): string {
  checkKubectlArgs(args, opts.input)
  const full = ['--context', liveContext(), '-n', LIVE_NAMESPACE, ...args]
  try {
    return execFileSync('kubectl', full, {
      encoding: 'utf8',
      input: opts.input ? JSON.stringify(opts.input) : undefined,
      stdio: ['pipe', 'pipe', 'pipe'],
      timeout: 120_000,
    })
  } catch (err) {
    const stderr = (err as { stderr?: string }).stderr?.trim() ?? ''
    throw new Error(`kubectl ${full.join(' ')} failed: ${stderr || String(err)}`, { cause: err })
  }
}

// eslint-disable-next-line @typescript-eslint/no-unnecessary-type-parameters -- typed JSON read
export function kubectlJson<T>(args: readonly string[]): T {
  return JSON.parse(kubectl([...args, '-o', 'json'])) as T
}

// ---------------------------------------------------------------------------- cluster truth

interface ContainerState {
  waiting?: { reason?: string }
  terminated?: { reason?: string }
  running?: unknown
}

export interface KubePod {
  metadata: {
    name: string
    namespace: string
    labels?: Record<string, string>
    deletionTimestamp?: string
    ownerReferences?: { kind: string; name: string }[]
  }
  spec?: { nodeName?: string }
  status?: {
    phase?: string
    podIP?: string
    containerStatuses?: {
      name: string
      ready: boolean
      restartCount: number
      state?: ContainerState
      lastState?: ContainerState
    }[]
  }
}

export interface KubeDeployment {
  metadata: { name: string }
  spec?: { replicas?: number }
  status?: { replicas?: number; readyReplicas?: number; updatedReplicas?: number }
}

/** Pods in the sandbox, optionally by label selector (e.g. `app=web`). */
export function listPods(selector?: string): KubePod[] {
  const args = ['get', 'pods', ...(selector ? ['-l', selector] : [])]
  return kubectlJson<{ items: KubePod[] }>(args).items
}

export function getDeployment(name: string): KubeDeployment {
  return kubectlJson<KubeDeployment>(['get', 'deployment', name])
}

/** `.status.phase`, which is what the UI's Status column shows today. */
export function podPhase(p: KubePod): string {
  return p.status?.phase ?? 'Unknown'
}

/**
 * The STATUS column of `kubectl get pods` (simplified): Terminating, else the first container
 * waiting/terminated reason (CrashLoopBackOff, ErrImagePull, Error, …), else the phase.
 */
export function kubectlStatus(p: KubePod): string {
  if (p.metadata.deletionTimestamp) return 'Terminating'
  for (const c of p.status?.containerStatuses ?? []) {
    const reason = c.state?.waiting?.reason ?? c.state?.terminated?.reason
    if (reason) return reason
  }
  return podPhase(p)
}

/** Names of live (not terminating) pods matching `selector`. */
export function podNames(selector?: string): string[] {
  return listPods(selector)
    .filter((p) => !p.metadata.deletionTimestamp)
    .map((p) => p.metadata.name)
    .sort()
}

// ---------------------------------------------------------------------------- polling

export interface WaitOptions {
  timeout?: number
  interval?: number
  message?: string
}

/**
 * Poll `cond` until it returns something other than `undefined`/`false`/`null`, and return it.
 * Errors thrown by `cond` count as "not yet" (the last one is reported on timeout).
 */
export async function waitFor<T>(
  cond: () => T | undefined | null | false | Promise<T | undefined | null | false>,
  { timeout = 60_000, interval = 1_000, message = 'condition' }: WaitOptions = {},
): Promise<T> {
  const deadline = Date.now() + timeout
  let lastError: unknown
  for (;;) {
    try {
      const value = await cond()
      if (value !== undefined && value !== null && value !== false) return value
    } catch (err) {
      lastError = err
    }
    if (Date.now() > deadline) {
      const detail = lastError instanceof Error ? ` (last error: ${lastError.message})` : ''
      throw new Error(
        `[live e2e] timed out after ${String(timeout)} ms waiting for ${message}${detail}`,
      )
    }
    await new Promise((r) => setTimeout(r, interval))
  }
}

/** Wait until `names` are all Running and ready. */
export function waitForPodsReady(names: readonly string[], timeout = 120_000): Promise<true> {
  return waitFor(
    () => {
      const byName = new Map(listPods().map((p) => [p.metadata.name, p]))
      return names.every((n) => {
        const p = byName.get(n)
        return (
          p !== undefined &&
          podPhase(p) === 'Running' &&
          (p.status?.containerStatuses ?? []).every((c) => c.ready)
        )
      })
    },
    { timeout, message: `pods ${names.join(', ')} to be ready` },
  )
}

/** Wait until none of `names` exists any more (terminating counts as existing). */
export function waitForPodsGone(names: readonly string[], timeout = 120_000): Promise<true> {
  return waitFor(
    () => {
      const present = new Set(listPods().map((p) => p.metadata.name))
      return names.every((n) => !present.has(n))
    },
    { timeout, message: `pods ${names.join(', ')} to be deleted` },
  )
}

// ---------------------------------------------------------------------------- sandbox objects

/** A restricted-PSA busybox pod like the ones in dev/sandbox/workloads.yaml. */
function throwawayPod(name: string): KubeObject {
  return {
    apiVersion: 'v1',
    kind: 'Pod',
    metadata: {
      name,
      namespace: LIVE_NAMESPACE,
      labels: { [E2E_LABEL[0]]: E2E_LABEL[1], app: 'e2e-throwaway' },
    },
    spec: {
      // `sleep` as PID 1 ignores SIGTERM: don't wait the default 30 s on delete.
      terminationGracePeriodSeconds: 1,
      securityContext: {
        runAsNonRoot: true,
        runAsUser: 65534,
        runAsGroup: 65534,
        seccompProfile: { type: 'RuntimeDefault' },
      },
      containers: [
        {
          name: 'main',
          image: 'busybox:1.37',
          command: ['sh', '-c', 'sleep infinity'],
          resources: {
            requests: { cpu: '5m', memory: '8Mi' },
            limits: { cpu: '20m', memory: '16Mi' },
          },
          securityContext: {
            allowPrivilegeEscalation: false,
            capabilities: { drop: ['ALL'] },
          },
        },
      ],
    },
  }
}

/** Create `n` throwaway pods labelled `fathom.dev/e2e=true`; returns their names. */
export function createThrowawayPods(n: number, prefix = 'e2e-throwaway'): string[] {
  const stamp = Date.now().toString(36)
  const names = Array.from({ length: n }, (_, i) => `${prefix}-${stamp}-${String(i + 1)}`)
  kubectl(['create', '-f', '-'], {
    input: { apiVersion: 'v1', kind: 'List', items: names.map(throwawayPod) },
  })
  return names
}

/**
 * Back to the committed baseline: scale-me at 1 replica, every `fathom.dev/e2e=true` object
 * gone. Safe to run after any spec (and when nothing changed).
 */
export function restoreBaseline(): void {
  const selector = `${E2E_LABEL[0]}=${E2E_LABEL[1]}`
  kubectl([
    'delete',
    'pods,deployments,configmaps',
    '-l',
    selector,
    '--ignore-not-found',
    '--wait=false',
  ])
  kubectl(['scale', 'deployment/scale-me', '--replicas=1'])
  kubectl(['rollout', 'status', 'deployment/scale-me', '--timeout=120s'])
}

// ---------------------------------------------------------------------------- the app

export interface LiveApp {
  /** The kube context the bridge is pinned to (`$FATHOM_LIVE_CONTEXT`). */
  readonly context: string
  /** `http://localhost:4174/?bridge=ws://127.0.0.1:<port>/&token=<t>` */
  readonly url: string
  /** Load the app and wait until it is connected to the bridge's context. */
  open(): Promise<void>
  selectContext(name?: string): Promise<void>
  /** A namespace, or `all`. */
  selectNamespace(name: string): Promise<void>
}

function bridgeUrl(): { port: string; token: string } {
  const port = process.env[ENV.port]
  const token = process.env[ENV.token]
  if (!port || !token) {
    throw new Error(
      '[live e2e] no devbridge: run through playwright.live.config.ts (pnpm e2e:live), ' +
        'whose global setup starts it.',
    )
  }
  return { port, token }
}

function makeApp(page: Page): LiveApp {
  const context = liveContext()
  const { port, token } = bridgeUrl()
  const url = `${APP_ORIGIN}/?${new URLSearchParams({
    bridge: `ws://127.0.0.1:${port}/`,
    token,
  }).toString()}`
  const picker = (label: string) => page.getByRole('combobox', { name: label })
  const pick = async (label: string, option: string) => {
    await picker(label).click()
    await page.getByRole('option', { name: option, exact: true }).click()
    await expect(picker(label)).toContainText(option)
  }
  return {
    context,
    url,
    async open() {
      await page.goto(url)
      await expect(picker('Context')).toContainText(context, { timeout: 60_000 })
      await expect(page.locator('[data-pane-state="disconnected"]')).toHaveCount(0)
    },
    selectContext: (name = context) => pick('Context', name),
    selectNamespace: (name) => pick('Namespace', name),
  }
}

/** Frames seen on the page's bridge socket (see `tapBridge`). */
export interface BridgeTap {
  /** Commands the page sent (after any rewrite). */
  readonly sent: BackendCmd[]
  /** Error replies (`Forbidden`, …) the bridge sent back. */
  readonly errors: Extract<DevBridgeServerMsg, { op: 'error' }>['error'][]
  /** Cluster writes only (deletes, scales, restarts, …), not reads or local commands. */
  writes(): BackendCmd[]
}

const WRITE_CMD =
  /^(Delete|BulkDelete|Scale|Restart|Rollback|Cordon|Uncordon|Drain|Evict|Apply|Create|Patch|Update|Edit|Suspend|Resume|Trigger)/

/**
 * Route the page's bridge WebSocket through Playwright (install BEFORE `app.open()`): every
 * frame still goes to the real bridge, and is recorded. `rewrite` may change a command first,
 * e.g. to point a write that must be refused at an object that does not exist, so a broken guard
 * could never touch a real object outside the sandbox.
 */
export async function tapBridge(
  page: Page,
  rewrite?: (cmd: BackendCmd) => BackendCmd,
): Promise<BridgeTap> {
  const tap = {
    sent: [] as BackendCmd[],
    errors: [] as BridgeTap['errors'],
    writes() {
      return this.sent.filter((c) => WRITE_CMD.test(c.type))
    },
  }
  await page.routeWebSocket(/^ws:\/\/127\.0\.0\.1:\d+\//, (ws: WebSocketRoute) => {
    const server = ws.connectToServer()
    ws.onMessage((raw) => {
      let msg = JSON.parse(String(raw)) as DevBridgeClientMsg
      if (msg.op === 'send') {
        msg = { op: 'send', cmd: rewrite ? rewrite(msg.cmd) : msg.cmd }
        tap.sent.push(msg.cmd)
      }
      server.send(JSON.stringify(msg))
    })
    server.onMessage((raw) => {
      const msg = JSON.parse(String(raw)) as DevBridgeServerMsg
      if (msg.op === 'error') tap.errors.push(msg.error)
      ws.send(raw)
    })
  })
  return tap
}

/** Tests get `app` (and the usual `page`); nothing about the bridge is stored on disk. */
export const test = base.extend<{ app: LiveApp }>({
  app: async ({ page }, provide) => {
    await provide(makeApp(page))
  },
})
export { expect }

// ---------------------------------------------------------------------------- UI lookups

/** Narrow `section`'s list to rows whose name starts with `query` (virtualised lists). */
export async function search(page: Page, query: string, section?: string): Promise<void> {
  await listPane(page, section).getByRole('searchbox').fill(query)
}

/** `fathom-dev/<name>` */
export const key = (name: string): string => `${LIVE_NAMESPACE}/${name}`

/** The row for `name` in `section`, found through the search box (lists are virtualised). */
export async function findRow(page: Page, name: string, section?: string): Promise<Locator> {
  await search(page, name, section)
  const r = row(page, key(name), section)
  await expect(r).toBeVisible()
  return r
}

/** The Status cell text of `row`. */
export function statusCell(r: Locator): Locator {
  return r.locator('[data-col="status"]')
}

/** Make `section`'s list visible (wide shows deployments beside pods already). */
export async function showList(page: Page, section: string, label: string): Promise<void> {
  if (!(await table(page, section).isVisible())) await navigateTo(page, label)
  await expect(table(page, section)).toBeVisible()
}

/** Row keys currently rendered in `section`'s table (only the visible, virtualised window). */
export function renderedKeys(page: Page, section?: string): Promise<string[]> {
  return table(page, section)
    .locator('[role="row"][data-row-key]')
    .evaluateAll((rows) => rows.map((r) => r.getAttribute('data-row-key') ?? ''))
}

/** `data-row-count` of `section`'s table (after search filtering), or -1 if not rendered. */
export async function rowCount(page: Page, section?: string): Promise<number> {
  const t = table(page, section)
  if ((await t.count()) === 0) return -1
  return Number((await t.getAttribute('data-row-count')) ?? -1)
}

// ---------------------------------------------------------------------------- US3 actions

export function actionToast(page: Page, type: 'success' | 'error' | 'loading'): Locator {
  return page.locator(`[data-testid="action-toast"][data-type="${type}"]`)
}
export const confirmDialog = (page: Page): Locator => page.getByTestId('confirm-dialog')
export const scaleDialog = (page: Page): Locator => page.getByTestId('scale-dialog')

/**
 * Tick the row checkboxes of `names` in `section`, after narrowing the list with `query` (by
 * default the first name; pass a shared prefix to tick several).
 */
export async function checkRows(
  page: Page,
  names: readonly string[],
  { query = names[0] ?? '', section }: { query?: string; section?: string } = {},
): Promise<void> {
  await search(page, query, section)
  for (const name of names) {
    const box = row(page, key(name), section).getByRole('checkbox', { name: `Select ${key(name)}` })
    await box.click()
    await expect(box).toHaveAttribute('aria-checked', 'true')
  }
}
