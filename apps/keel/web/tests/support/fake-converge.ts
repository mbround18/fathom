/**
 * The fake backend's converge (spec 068 T029), answering as contracts/protocol.md describes, on a
 * `FakeBackend` with `on(...)` so the shared fake stays untouched:
 *
 * - `PreviewConverge(ConvergeTarget)` answers `ConvergePlan { preview_token, plan }`, the plan from
 *   `planFor` (by default every targeted host runs `steps`), and records the target.
 * - `ConfirmConverge { preview_token }` starts a parent run (kind `converge`, in FakeRuns, held for
 *   approval when the plan's policy asks) with one child run per step, and answers
 *   `ConvergeStarted { run_id }`. A token it did not issue answers `OpError preview_stale`.
 * - `GetConverge { run_id }` answers `Converge(ConvergeView)`.
 * - `CancelConverge` cancels each blocked or queued step and pushes `ConvergeChanged`.
 * - `PreviewConvergeRetry { run_id, replan }` plans the steps that did not succeed;
 *   `ConfirmConvergeRetry` starts a new converge with `retry_of`.
 * - `step(run, host, position, state)` moves one step as the server would and pushes
 *   `ConvergeChanged`.
 *
 * The converge commands and events are not in the generated `Cmd`/`Event` unions yet (the names
 * live in src/features/converge/wire.ts), so this file casts at the edge, like that module does.
 */
import type {
  ConvergeHostState,
  ConvergePlan,
  ConvergeStepView,
  ConvergeTarget,
  ConvergeView,
  Event,
  HostPlan,
  PlannedStep,
  StepState,
} from '../../src/protocol/wire'
import { CMD, EVENT, type ConvergeCmd, type ConvergeEvent } from '../../src/features/converge/wire'
import type { FakeBackend, Reply } from './fake-backend'
import type { FakeHosts } from './fake-hosts'
import { HEAD, NOW } from './fake-repos'
import { runInfo, type FakeRuns } from './fake-runs'

export interface StepSpec {
  job: string
  via?: string[]
  params?: Record<string, string>
  credential_names?: string[]
}

export function plannedSteps(steps: readonly StepSpec[]): PlannedStep[] {
  return steps.map((s, i) => ({
    position: i + 1,
    job: s.job,
    via: s.via ?? [],
    params: s.params ?? {},
    credential_names: s.credential_names ?? [],
  }))
}

export function hostPlan(host_id: string, host: string, steps: readonly StepSpec[], over: Partial<HostPlan> = {}): HostPlan {
  return { host_id, host, profiles: ['base'], steps: plannedSteps(steps), refusals: [], ...over }
}

export function convergePlan(over: Partial<ConvergePlan> = {}): ConvergePlan {
  return {
    binding_id: 'binding-ops',
    commit: HEAD,
    inventory_revision: 'rev-0003',
    check: false,
    hosts: [],
    policy: { requires_approval: false, approvals_needed: 0, four_eyes: false, approval_ttl_secs: 14_400 },
    review: [],
    replaces: [],
    blocked_by: [],
    ...over,
  }
}

type RawHandler = (cmd: ConvergeCmd) => Reply
type RawOn = (type: string, handler: RawHandler) => FakeBackend

const ENDED: readonly ConvergeHostState[] = ['succeeded', 'failed', 'cancelled']

/** A host's state from its steps. */
function hostState(steps: readonly ConvergeStepView[]): ConvergeHostState {
  if (steps.some((s) => s.state === 'failed' || s.state === 'lost' || s.state === 'refused')) return 'failed'
  if (steps.every((s) => s.state === 'succeeded' || s.state === 'covered')) return 'succeeded'
  if (steps.some((s) => s.state === 'cancelled')) return 'cancelled'
  if (steps.some((s) => s.state === 'sent' || s.state === 'running')) return 'holding'
  return 'waiting'
}

export class FakeConverge {
  /** The plan each preview answers. Default: every targeted host runs `steps`. */
  planFor: (target: ConvergeTarget) => ConvergePlan
  /** The steps the default plan gives each host. */
  steps: StepSpec[] = [{ job: 'debian' }, { job: 'keel-node', via: ['keel-node'] }]
  /** Every target previewed, in order. */
  readonly previewed: ConvergeTarget[] = []
  readonly views = new Map<string, ConvergeView>()
  private readonly tokens = new Map<string, { plan: ConvergePlan; retry_of: string | null }>()
  private seq = 0

  private readonly backend: FakeBackend
  private readonly runs: FakeRuns
  private readonly workspace: string

  constructor(backend: FakeBackend, runs: FakeRuns, hosts: FakeHosts, workspace: string) {
    this.backend = backend
    this.runs = runs
    this.workspace = workspace
    this.planFor = (target) => {
      const picked = target.hosts.kind === 'all' ? hosts.hosts : hosts.hosts.filter((h) => target.hosts.kind === 'hosts' && target.hosts.ids.includes(h.id))
      return convergePlan({
        binding_id: target.binding_id,
        check: target.check,
        hosts: picked.map((h) => hostPlan(h.id, h.inventory_name, this.steps, { profiles: h.profiles })),
      })
    }
    const on = backend.on.bind(backend) as unknown as RawOn
    for (const type of Object.values(CMD)) on(type, (cmd) => this.respond(cmd))
  }

  /** Moves one step and pushes `ConvergeChanged` for its host. */
  step(run_id: string, host: string, position: number, state: StepState): void {
    const view = this.views.get(run_id)
    if (!view) throw new Error(`no converge ${run_id}`)
    const h = view.hosts.find((x) => x.host === host)
    if (!h) throw new Error(`no host ${host} in ${run_id}`)
    h.steps = h.steps.map((s) => (s.position === position ? { ...s, state } : s))
    h.state = hostState(h.steps)
    this.push({ type: EVENT.changed, data: { run_id, host_id: h.host_id, state: h.state } })
  }

  private push(e: ConvergeEvent): void {
    this.backend.push(e)
  }

  private reply(...events: ConvergeEvent[]): Event[] {
    return events
  }

  private issue(plan: ConvergePlan, retry_of: string | null): Event[] {
    const preview_token = `ptok-${String(++this.seq)}`
    this.tokens.set(preview_token, { plan, retry_of })
    return this.reply({ type: EVENT.plan, data: { preview_token, plan } })
  }

  /** Starts a converge of `plan` as Confirm would; returns the parent run id. */
  begin(plan: ConvergePlan, retry_of: string | null = null): string {
    const id = `conv-${String(++this.seq)}`
    const held = plan.policy.requires_approval
    const hosts = plan.hosts.filter((h) => h.steps.length > 0 && h.refusals.length === 0)
    this.runs.add(
      runInfo({
        id,
        workspace: this.workspace,
        binding: plan.binding_id,
        job: 'converge',
        job_name: 'converge',
        trigger: 'manual',
        kind: 'converge',
        check: plan.check,
        state: held ? 'pending_approval' : 'running',
        hosts: hosts.map((h) => h.host),
        started_at: held ? null : NOW,
      }),
    )
    const view: ConvergeView = {
      run: runInfo({ id, workspace: this.workspace, kind: 'converge', check: plan.check }),
      retry_of,
      hosts: hosts.map((h) => {
        const steps: ConvergeStepView[] = h.steps.map((s) => {
          const child = `${id}-${h.host}-${String(s.position)}`
          this.runs.add(
            runInfo({ id: child, workspace: this.workspace, job_name: s.job, kind: 'converge', parent_run: id, check: plan.check, hosts: [h.host], state: 'queued', started_at: null }),
          )
          return {
            position: s.position,
            job: s.job,
            run_id: child,
            state: s.position === 1 && !held ? 'queued' : 'blocked',
            reason: null,
            started_at: null,
            ended_at: null,
          }
        })
        return { host_id: h.host_id, host: h.host, state: 'waiting', steps }
      }),
    }
    this.views.set(id, view)
    return id
  }

  private respond(cmd: ConvergeCmd): Reply {
    switch (cmd.type) {
      case CMD.preview:
        this.previewed.push(cmd.data)
        return this.issue(this.planFor(cmd.data), null)
      case CMD.confirm:
      case CMD.confirmRetry: {
        const issued = this.tokens.get(cmd.data.preview_token)
        if (!issued) return [{ type: 'OpError', data: { code: 'preview_stale', message: 'The plan changed. Preview again.' } }]
        this.tokens.delete(cmd.data.preview_token)
        return this.reply({ type: EVENT.started, data: { run_id: this.begin(issued.plan, issued.retry_of) } })
      }
      case CMD.get: {
        const view = this.views.get(cmd.data.run_id)
        if (!view) return [{ type: 'OpError', data: { code: 'not_found', message: 'No such converge.' } }]
        return this.reply({ type: EVENT.view, data: view })
      }
      case CMD.cancel: {
        const view = this.views.get(cmd.data.run_id)
        if (!view) return [{ type: 'OpError', data: { code: 'not_found', message: 'No such converge.' } }]
        const out: ConvergeEvent[] = []
        for (const h of view.hosts) {
          h.steps = h.steps.map((s) => (s.state === 'blocked' || s.state === 'queued' ? { ...s, state: 'cancelled' } : s))
          h.state = hostState(h.steps)
          out.push({ type: EVENT.changed, data: { run_id: view.run.id, host_id: h.host_id, state: h.state } })
        }
        return this.reply(...out)
      }
      case CMD.previewRetry: {
        const view = this.views.get(cmd.data.run_id)
        if (!view) return [{ type: 'OpError', data: { code: 'not_found', message: 'No such converge.' } }]
        const hosts = view.hosts
          .filter((h) => ENDED.includes(h.state) && h.state !== 'succeeded')
          .map((h) =>
            hostPlan(
              h.host_id,
              h.host,
              h.steps.filter((s) => s.state !== 'succeeded' && s.state !== 'covered').map((s) => ({ job: s.job })),
            ),
          )
        return this.issue(convergePlan({ hosts, commit: cmd.data.replan ? 'f'.repeat(40) : HEAD }), cmd.data.run_id)
      }
    }
  }
}
