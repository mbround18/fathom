/**
 * Spec 062 US2–US5 in the fake backend: batch previews and runs, and every `UpdateChange`.
 *
 * - `PreviewUpdateBatch` answers `UpdatePreview`, one plan per stack (newer services only, the
 *   rest skipped with a reason, the 051 findings of `lint`).
 * - Its `Confirm` answers `UpdateRunStarted`, then pushes the run as the server does: per stack
 *   `OpStarted`, `OpOutput`, `OpFinished` and `UpdateStackDone` (a busy or stale stack gets no
 *   operation), then `UpdateRunFinished`. Each stack follows `scripts[stack]`; `pauseBefore` holds
 *   the run before that stack's `OpFinished` until `resume()` (or a `Cancel`).
 * - `PreviewUpdateChange` answers a `Preview` whose `file_diff` is the server's summary; its
 *   `Confirm` applies the change, audits it and answers `UpdatesChanged`. An apply-anyway starts a
 *   one-stack run instead.
 */
import type {
  Actor,
  AuditEntry,
  ComposeWarning,
  Cmd,
  Event,
  MaintenanceWindow,
  Preview,
  ServiceUpdate,
  StackOutcome,
  StackRun,
  StackUpdatePlan,
  UpdateChange,
  UpdateRun,
  UpdateSettings,
  UpdatesView,
} from '../../src/protocol/generated'
import type { FakePasskeys } from './fake-passkeys'

type Audit = Partial<AuditEntry> & Pick<AuditEntry, 'action' | 'outcome'>

/** What one stack does when a run reaches it. */
export interface StackScript {
  /** Default `updated`. `held` uses `hold` as the reason. */
  outcome?: StackOutcome
  /** Output lines; a default pull/recreate/health transcript when unset. */
  output?: string[]
  /** The 051 code or guard rule of a `held` outcome. */
  hold?: string
  /** stderr of a failed or rolled-back stack, kept in its audit entry. */
  stderr?: string
  /** Replaced images removed after the update (cleanup on). */
  removed?: { id: string; digests: string[] }[]
  /** Replaced images another container still uses. */
  keptInUse?: { id: string; why: string }[]
}

export interface UpdatesHost {
  view: () => UpdatesView | null
  setView: (v: UpdatesView) => void
  actor: () => Actor
  admin: () => boolean
  audit: (over: Audit) => void
  passkeys: FakePasskeys
  nextId: () => string
  lint: (stack: string) => ComposeWarning[]
  enforce: () => boolean
  push: (e: Event) => void
  /** When the run's events go out: `NOW` keeps audit entries inside the run's window. */
  now: string
}

type Pending =
  | { kind: 'batch'; stacks: string[]; plans: StackUpdatePlan[] }
  | { kind: 'change'; change: UpdateChange }

interface Live {
  runId: string
  stacks: string[]
  cancelled: boolean
  current: string | null
  resume: (() => void) | null
}

const STEP_MS = 40

const word = (b: boolean): string => (b ? 'on' : 'off')
const serverWindow = (w: MaintenanceWindow): string => `${w.days.join(',')} ${w.start}-${w.end}`

/** The settings summary as `updates::change::summary` writes it. */
export function settingsSummary(before: UpdateSettings, after: UpdateSettings): string {
  const lines: string[] = []
  if (before.checks_enabled !== after.checks_enabled)
    lines.push(`update checks: ${word(before.checks_enabled)} -> ${word(after.checks_enabled)}`)
  if (before.interval_secs !== after.interval_secs)
    lines.push(`check interval: every ${String(before.interval_secs / 3600)} h -> every ${String(after.interval_secs / 3600)} h`)
  if (JSON.stringify(before.window) !== JSON.stringify(after.window))
    lines.push(`maintenance window: ${serverWindow(before.window)} -> ${serverWindow(after.window)}`)
  if (before.health_timeout_secs !== after.health_timeout_secs)
    lines.push(`health timeout: ${String(before.health_timeout_secs)} s -> ${String(after.health_timeout_secs)} s`)
  if (before.cleanup !== after.cleanup) lines.push(`remove replaced images: ${word(before.cleanup)} -> ${word(after.cleanup)}`)
  if (before.label_only !== after.label_only)
    lines.push(`only services with the Watchtower enable label: ${word(before.label_only)} -> ${word(after.label_only)}`)
  if (before.scope !== after.scope) {
    const scope = (s: string | undefined): string => (s === undefined ? 'none set' : `\`${s}\``)
    lines.push(`Watchtower scope: ${scope(before.scope)} -> ${scope(after.scope)}`)
  }
  return lines.join('\n')
}

export class FakeUpdates {
  /** Per stack: what a run does there. */
  scripts: Record<string, StackScript> = {}
  /** Stacks another operation holds: `busy: <actor> <kind>`. */
  busy: Record<string, string> = {}
  /** Stacks whose compose file changes between preview and Confirm. */
  stale = new Set<string>()
  /** The run waits before this stack's `OpFinished` until `resume()` or a `Cancel`. */
  pauseBefore: string | null = null

  private readonly host: UpdatesHost
  private readonly pending = new Map<string, Pending>()
  private live: Live | null = null

  private current(): UpdatesView {
    const v = this.host.view()
    if (v === null) throw new Error('the fake has no updates view')
    return v
  }

  constructor(host: UpdatesHost) {
    this.host = host
  }

  /** Whether `Confirm{id}` is ours. */
  owns(previewId: string): boolean {
    return this.pending.has(previewId)
  }

  /** The current run's operation for `operationId`, if any. */
  ownsOp(operationId: string): boolean {
    return this.live !== null && this.live.current === operationId
  }

  /** Lets a paused run go on. */
  resume(): void {
    const live = this.live
    const r = live?.resume
    if (live && r) {
      live.resume = null
      r()
    }
  }

  /** Stop, as the server does it: the stack in progress finishes, the stacks after it are skipped. */
  cancel(operationId: string): Event[] {
    if (!this.ownsOp(operationId)) return [{ type: 'Invalid', data: { reason: 'no such operation' } }]
    if (this.live) this.live.cancelled = true
    this.resume()
    return []
  }

  previewBatch(stacks: string[]): Event[] {
    const view = this.host.view()
    if (view === null) return [{ type: 'Denied', data: { reason: 'feature_disabled' } }]
    if (!this.host.admin()) return [{ type: 'Denied', data: { reason: 'not_permitted' } }]
    if (stacks.length === 0) return [{ type: 'Invalid', data: { reason: 'choose at least one stack' } }]
    const twice = stacks.find((s, i) => stacks.indexOf(s) !== i)
    if (twice) return [{ type: 'Invalid', data: { reason: `${twice} is listed twice` } }]
    const plans = stacks.map((stack) => this.plan(view, stack))
    const preview_id = this.previewId()
    this.pending.set(preview_id, { kind: 'batch', stacks, plans })
    const step_up = this.host.passkeys.stampId(preview_id)
    return [{ type: 'UpdatePreview', data: { preview_id, stacks: plans, ...(step_up ? { step_up } : {}) } }]
  }

  private plan(view: UpdatesView, stack: string): StackUpdatePlan {
    const services = view.services.filter((s) => s.stack === stack && s.state === 'available')
    const skipped = view.services
      .filter((s) => s.stack === stack && s.state !== 'available')
      .map((s): [string, string] => [s.service, s.state === 'up_to_date' ? 'up to date' : (s.reason ?? s.state.replace('_', ' '))])
    const findings = this.host.lint(stack)
    const argv = [
      ...services.map((s) => ['docker', 'pull', `${s.reference.split(':')[0] ?? s.reference}@${s.available?.digest ?? ''}`]),
      ...services.map((s) => ['docker', 'image', 'tag', s.available?.digest ?? '', s.reference]),
      ['docker', 'compose', '-p', stack, 'up', '-d', '--no-deps', ...services.map((s) => s.service)],
    ]
    return {
      stack,
      services,
      skipped,
      argv,
      findings,
      ack_required: this.host.enforce() && findings.some((f) => f.severity === 'escape'),
    }
  }

  previewChange(change: UpdateChange): Event[] {
    const view = this.host.view()
    if (view === null) return [{ type: 'Denied', data: { reason: 'feature_disabled' } }]
    if (!this.host.admin()) return [{ type: 'Denied', data: { reason: 'not_permitted' } }]
    const made = this.summary(view, change)
    if (typeof made !== 'string') return [{ type: 'Invalid', data: { reason: made.invalid } }]
    const preview_id = this.previewId()
    this.pending.set(preview_id, { kind: 'change', change })
    const stack = change.kind === 'policy' || change.kind === 'retry' || change.kind === 'apply_held' ? change.data.stack : ''
    const steps =
      change.kind === 'apply_held'
        ? this.plan(view, stack).argv.map((argv) => ({ argv, cwd: `/stacks/${stack}`, stdin: 'none' as const }))
        : []
    const preview: Preview = {
      preview_id,
      stack,
      kind: 'update',
      steps,
      file_diff: made,
      reaches: { containers: [], networks: [], volumes: [] },
      destructive: false,
      stack_hash: '',
      expires_at: '2026-10-28T12:05:00Z',
    }
    return [{ type: 'Preview', data: this.host.passkeys.stamp(preview) }]
  }

  private summary(view: UpdatesView, change: UpdateChange): string | { invalid: string } {
    switch (change.kind) {
      case 'settings': {
        const after = change.data
        if (after.interval_secs < 3600)
          return { invalid: `interval_secs is ${String(after.interval_secs)}; the minimum is 3600 (one hour)` }
        const s = settingsSummary(view.settings, after)
        return s === '' ? { invalid: 'nothing to change' } : s
      }
      case 'policy': {
        const { stack, policy, window } = change.data
        const before = view.policies[stack]
        const paused = view.paused_policies.includes(stack)
        if (before && before.policy === policy && JSON.stringify(before.window ?? null) === JSON.stringify(window ?? null) && !paused)
          return { invalid: 'nothing to change' }
        const lines = [`update policy of ${stack}: ${before ? before.policy : 'notify (default)'} -> ${policy}`]
        const old = before?.window ?? null
        if (JSON.stringify(old) !== JSON.stringify(window ?? null)) {
          const show = (w: MaintenanceWindow | null | undefined): string => (w ? serverWindow(w) : 'the host window')
          lines.push(`maintenance window: ${show(old)} -> ${show(window)}`)
        }
        if (policy === 'auto') lines.push(`Depths will install new images for ${stack} in the maintenance window, on your behalf`)
        return lines.join('\n')
      }
      case 'retry': {
        const { stack, service, digest } = change.data
        const s = view.services.find((x) => x.stack === stack && x.service === service && x.state === 'rejected')
        if (!s) return { invalid: `${stack}/${service} has no rejected image ${digest}` }
        return `forget that ${digest} failed for ${stack}/${service}: the next check offers it again`
      }
      case 'apply_held': {
        const { stack, service, digest } = change.data
        const s = view.services.find((x) => x.stack === stack && x.service === service && x.state === 'held')
        if (!s) return { invalid: `${stack}/${service}: it is not held` }
        return `install ${digest} for ${stack}/${service} although it was held; the hold is overridden for this run only and audited`
      }
      case 'ack_watchtower': {
        const w = view.watchtower.find((x) => x.id.startsWith(change.data.container_id))
        if (!w) return { invalid: `no running Watchtower container ${change.data.container_id}` }
        if (w.acknowledged === change.data.answer) return { invalid: 'nothing to change' }
        const says =
          change.data.answer === 'stopped'
            ? 'you stopped it, or will stop it'
            : 'it only monitors (WATCHTOWER_MONITOR_ONLY), so it changes nothing'
        return `Watchtower container ${w.name} (${w.id}): ${says}; the warning is no longer shown for it`
      }
    }
  }

  /** `Confirm` of a batch or change preview. */
  confirm(cmd: Extract<Cmd, { type: 'Confirm' }>): Event[] {
    const p = this.pending.get(cmd.data.preview_id)
    if (!p) return [{ type: 'PreviewVoid', data: { reason: 'unknown' } }]
    this.pending.delete(cmd.data.preview_id)
    const view = this.host.view()
    if (view === null) return [{ type: 'Denied', data: { reason: 'feature_disabled' } }]
    if (p.kind === 'batch') {
      if (p.plans.some((x) => x.ack_required) && cmd.data.acknowledge_findings !== true)
        return [{ type: 'Denied', data: { reason: 'compose_blocked', detail: 'acknowledge the findings to update' } }]
      if (p.plans.every((x) => x.services.length === 0))
        return [{ type: 'Invalid', data: { reason: 'nothing to update: every service shown is up to date or left as it is' } }]
      return this.start(p.stacks, { kind: 'manual', actor: this.host.actor() })
    }
    return this.apply(view, p.change)
  }

  private apply(view: UpdatesView, change: UpdateChange): Event[] {
    const actor = this.host.actor()
    switch (change.kind) {
      case 'settings':
        this.host.audit({ action: 'update_settings', outcome: { kind: 'succeeded' }, change: { before: view.settings, after: change.data } })
        this.set({ ...view, settings: change.data })
        break
      case 'policy': {
        const { stack, policy, window } = change.data
        const before = view.policies[stack] ?? null
        const policies = { ...view.policies, [stack]: { policy, window: window ?? null, set_by: actor, set_at: this.host.now } }
        const services = view.services.map((s) =>
          s.stack === stack && s.effective.excluded === null && s.effective.source.kind !== 'label'
            ? { ...s, effective: { ...s.effective, policy, source: { kind: 'stack_policy' as const } } }
            : s,
        )
        this.host.audit({ action: 'update_policy', stack, outcome: { kind: 'succeeded' }, change: { before, after: policies[stack] } })
        this.set({ ...view, policies, services, paused_policies: view.paused_policies.filter((s) => s !== stack) })
        break
      }
      case 'retry': {
        const { stack, service, digest } = change.data
        this.host.audit({ action: 'update_retry', stack, outcome: { kind: 'succeeded' }, diff_summary: `forget ${digest}` })
        this.set({
          ...view,
          services: view.services.map((s) =>
            s.stack === stack && s.service === service ? { ...s, state: 'not_checked' as const, reason: null } : s,
          ),
        })
        break
      }
      case 'ack_watchtower': {
        const watchtower = view.watchtower.map((w) =>
          w.id.startsWith(change.data.container_id) ? { ...w, acknowledged: change.data.answer } : w,
        )
        const open = watchtower.filter((w) => w.acknowledged === null)
        const auto_paused =
          view.auto_paused?.kind === 'watchtower_running'
            ? open.length === 0
              ? null
              : { kind: 'watchtower_running' as const, containers: open.map((w) => w.name) }
            : view.auto_paused
        this.host.audit({ action: 'watchtower_ack', outcome: { kind: 'succeeded' }, change: { before: null, after: change.data } })
        this.set({ ...view, watchtower, auto_paused })
        break
      }
      case 'apply_held': {
        const { stack, service, digest } = change.data
        this.host.audit({
          action: 'image_hold_apply',
          stack,
          outcome: { kind: 'succeeded' },
          diff_summary: `overridden for ${digest}`,
        })
        // The hold is overridden for this run: the service is offered as available.
        this.set({
          ...view,
          services: view.services.map((s) =>
            s.stack === stack && s.service === service ? { ...s, state: 'available' as const, reason: null } : s,
          ),
        })
        const script = this.scripts[stack]
        if (script?.outcome?.kind === 'held') {
          const rest = { ...script }
          delete rest.outcome
          this.scripts[stack] = rest
        }
        return this.start([stack], { kind: 'manual', actor })
      }
    }
    return [{ type: 'UpdatesChanged', data: this.current() }]
  }

  private set(v: UpdatesView): void {
    this.host.setView(v)
  }

  private start(stacks: string[], trigger: UpdateRun['trigger']): Event[] {
    const runId = `00000000-0000-4000-9062-${this.host.nextId().padStart(12, '0')}`
    const live: Live = { runId, stacks, cancelled: false, current: null, resume: null }
    this.live = live
    setTimeout(() => void this.run(live, trigger), STEP_MS)
    return [{ type: 'UpdateRunStarted', data: { run_id: runId, trigger, stacks } }]
  }

  private async run(live: Live, trigger: UpdateRun['trigger']): Promise<void> {
    const wait = (ms = STEP_MS): Promise<void> => new Promise((r) => setTimeout(r, ms))
    const summary: [string, StackOutcome][] = []
    const runs: StackRun[] = []
    const before = this.host.view()
    for (const stack of live.stacks) {
      const services = (before?.services ?? []).filter((s) => s.stack === stack && s.state === 'available')
      const changes = services.map((s) => ({
        service: s.service,
        from_image: s.running?.image_id ?? '',
        to_digest: s.available?.digest ?? '',
        to_image: null,
      }))
      let outcome: StackOutcome
      let operationId: string | null = null
      if (live.cancelled) outcome = { kind: 'skipped', reason: 'cancelled' }
      else if (this.busy[stack]) outcome = { kind: 'skipped', reason: this.busy[stack] }
      else if (this.stale.has(stack)) outcome = { kind: 'skipped', reason: 'stale: preview again' }
      else {
        const script = this.scripts[stack] ?? {}
        operationId = this.host.nextId()
        live.current = operationId
        this.host.push({ type: 'OpStarted', data: { operation_id: operationId } })
        await wait()
        const lines = script.output ?? [
          ...services.map((s) => `pull ${s.reference}@${s.available?.digest ?? ''}: done\n`),
          `recreate ${services.map((s) => s.service).join(', ')}\n`,
          'health: healthy\n',
        ]
        for (const chunk of lines) {
          this.host.push({ type: 'OpOutput', data: { operation_id: operationId, chunk } })
          await wait(10)
        }
        if (this.pauseBefore === stack) {
          await new Promise<void>((r) => {
            live.resume = r
          })
        }
        outcome = script.outcome ?? { kind: 'updated' }
        if (outcome.kind === 'updated') {
          for (const k of script.keptInUse ?? []) {
            this.host.push({ type: 'OpOutput', data: { operation_id: operationId, chunk: `kept: in use: ${k.id} (${k.why})\n` } })
            await wait(10)
          }
        }
        this.host.push({
          type: 'OpFinished',
          data: {
            operation_id: operationId,
            outcome:
              outcome.kind === 'updated'
                ? { kind: 'succeeded' }
                : { kind: 'failed', exit_code: null, stderr: script.stderr ?? outcome.kind },
          },
        })
        live.current = null
        this.record(stack, outcome, script, services, trigger, live.runId)
      }
      this.after(stack, outcome)
      summary.push([stack, outcome])
      runs.push({ stack, services: changes, operation_id: operationId, phase: 'done', outcome })
      this.host.push({ type: 'UpdateStackDone', data: { run_id: live.runId, stack, outcome, audit_at: this.host.now } })
      await wait()
    }
    const view = this.host.view()
    if (view) {
      const run: UpdateRun = { run_id: live.runId, trigger, stacks: runs, started_at: this.host.now, finished_at: this.host.now }
      this.set({ ...view, recent_runs: [run, ...view.recent_runs] })
      this.host.push({ type: 'UpdatesChanged', data: this.current() })
    }
    this.host.push({ type: 'UpdateRunFinished', data: { run_id: live.runId, summary } })
    if (this.live === live) this.live = null
  }

  private record(
    stack: string,
    outcome: StackOutcome,
    script: StackScript,
    services: ServiceUpdate[],
    trigger: UpdateRun['trigger'],
    runId: string,
  ): void {
    const update = {
      run_id: runId,
      services: services.map((s) => ({
        service: s.service,
        from_image: s.running?.image_id ?? '',
        from_digest: s.running?.repo_digest ?? null,
        to_image: null,
        to_digest: s.available?.digest ?? '',
        version: s.available?.version ?? null,
      })),
      rejected_digest: outcome.kind === 'rolled_back' || outcome.kind === 'failed' ? (services[0]?.available?.digest ?? null) : null,
      hold: outcome.kind === 'held' ? (script.hold ?? null) : null,
    }
    const on_behalf_of = trigger.kind === 'scheduled' ? trigger.on_behalf_of[stack] : undefined
    const base = { stack, update, ...(on_behalf_of ? { on_behalf_of } : {}) }
    const failed = { kind: 'failed' as const, exit_code: null, stderr_tail: script.stderr ?? '' }
    switch (outcome.kind) {
      case 'updated':
        this.host.audit({ ...base, action: 'image_update', outcome: { kind: 'succeeded' } })
        for (const r of script.removed ?? [])
          this.host.audit({
            stack,
            action: 'image_remove',
            outcome: { kind: 'succeeded' },
            argv: [['docker', 'image', 'rm', r.id]],
            diff_summary: `removed ${r.id} (${r.digests.join(', ')})`,
            ...(on_behalf_of ? { on_behalf_of } : {}),
          })
        break
      case 'held':
        this.host.audit({ ...base, action: 'image_hold', outcome: { kind: 'denied', reason: 'compose_blocked' } })
        break
      case 'rolled_back':
      case 'rollback_failed':
        this.host.audit({ ...base, action: 'image_rollback', outcome: failed })
        break
      case 'interrupted':
        this.host.audit({ ...base, action: 'update_interrupted', outcome: failed })
        break
      case 'failed':
        this.host.audit({ ...base, action: 'image_update', outcome: failed })
        break
      default:
        break
    }
  }

  /** The view after a stack's outcome: updated is up to date, held waits, a failure is rejected. */
  private after(stack: string, outcome: StackOutcome): void {
    const view = this.host.view()
    if (!view) return
    const next = (s: ServiceUpdate): ServiceUpdate => {
      if (s.stack !== stack || s.state !== 'available') return s
      switch (outcome.kind) {
        case 'updated':
          return {
            ...s,
            state: 'up_to_date',
            running: s.running && s.available ? { ...s.running, repo_digest: s.available.digest, version: s.available.version } : s.running,
            available: null,
            last_outcome: outcome,
          }
        case 'held':
          return { ...s, state: 'held', reason: this.scripts[stack]?.hold ?? 'held', last_outcome: outcome }
        case 'rolled_back':
        case 'failed':
          return { ...s, state: 'rejected', reason: 'the new image failed its health check and was rolled back', last_outcome: outcome }
        case 'rollback_failed':
          return { ...s, state: 'rollback_failed', last_outcome: outcome }
        default:
          return s
      }
    }
    this.set({ ...view, services: view.services.map(next) })
  }

  private previewId(): string {
    return `00000000-0000-4000-8062-${this.host.nextId().padStart(12, '0')}`
  }
}
