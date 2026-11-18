/**
 * What the converge screens show, worked out from the plan and the view (spec 068
 * contracts/ui.md). Pure functions, so the matrix and the wording are tested without a page.
 */
import type {
  ApprovalTerms,
  ConvergeHostState,
  ConvergeHostSummary,
  ConvergePlan,
  ConvergeStepView,
  ConvergeView,
  HostInfo,
  HostPlan,
  Me,
  PlannedStep,
  StepRefusalReason,
  StepState,
} from '@/protocol/wire'

function plural(n: number, one: string, many = `${one}s`): string {
  return `${String(n)} ${n === 1 ? one : many}`
}

// --- The plan (preview) -------------------------------------------------------------------------

/** A host Confirm includes: it has steps and nothing refused it. */
export function included(host: HostPlan): boolean {
  return host.steps.length > 0 && host.refusals.length === 0
}

export interface PlanRow {
  host: HostPlan
  /** One per column position; `null` where the host's plan is shorter. */
  cells: (PlannedStep | null)[]
  included: boolean
}

export interface Matrix<R> {
  /** Column positions, 1-based. */
  positions: number[]
  rows: R[]
}

function columns(lengths: readonly number[]): number[] {
  const n = Math.max(0, ...lengths)
  return Array.from({ length: n }, (_, i) => i + 1)
}

/** Rows are hosts, columns are step positions. */
export function planMatrix(plan: ConvergePlan): Matrix<PlanRow> {
  const positions = columns(plan.hosts.map((h) => h.steps.length))
  const rows = plan.hosts.map((host) => {
    const at = new Map(host.steps.map((s) => [s.position, s]))
    return { host, cells: positions.map((p) => at.get(p) ?? null), included: included(host) }
  })
  return { positions, rows }
}

/** "from keel-node → debian": the profiles that brought the step in, outermost first. */
export function viaText(step: PlannedStep): string {
  return step.via.length === 0 ? 'assigned directly' : `from ${step.via.join(' → ')}`
}

export interface JobSteps {
  job: string
  params: [string, string][]
  credential_names: string[]
  /** How many included hosts run it. */
  hosts: number
}

/** Each job once, with its params and credential names (never values), in first-seen order. */
export function jobsOf(plan: ConvergePlan): JobSteps[] {
  const out = new Map<string, JobSteps>()
  for (const host of plan.hosts) {
    if (!included(host)) continue
    for (const s of host.steps) {
      const seen = out.get(s.job)
      if (seen) {
        seen.hosts += 1
        continue
      }
      out.set(s.job, {
        job: s.job,
        params: Object.entries(s.params).sort(([a], [b]) => a.localeCompare(b)),
        credential_names: [...s.credential_names].sort(),
        hosts: 1,
      })
    }
  }
  return [...out.values()]
}

/** Included hosts and their steps, for Confirm's label. */
export function planCounts(plan: ConvergePlan): { hosts: number; steps: number } {
  const hosts = plan.hosts.filter(included)
  return { hosts: hosts.length, steps: hosts.reduce((n, h) => n + h.steps.length, 0) }
}

/** "Converge 3 hosts (7 steps)". */
export function confirmLabel(plan: ConvergePlan): string {
  const { hosts, steps } = planCounts(plan)
  return `Converge ${plural(hosts, 'host')} (${plural(steps, 'step')})`
}

/** "4 h", "30 min", "2 d". */
export function ttlText(secs: number): string {
  if (secs >= 86_400 && secs % 86_400 === 0) return `${String(secs / 86_400)} d`
  if (secs >= 3_600) return `${String(Math.round((secs / 3_600) * 10) / 10)} h`
  if (secs >= 60) return `${String(Math.round(secs / 60))} min`
  return `${String(secs)} s`
}

/** "Needs 2 approvals, four eyes, expires in 4 h (strictest of 3 steps)". */
export function policyText(terms: ApprovalTerms, distinctJobs: number): string {
  if (!terms.requires_approval) return 'No approval needed: it starts on Confirm.'
  const parts = [`Needs ${plural(terms.approvals_needed, 'approval')}`]
  if (terms.four_eyes) parts.push('four eyes')
  parts.push(`expires in ${ttlText(terms.approval_ttl_secs)}`)
  const text = parts.join(', ')
  return distinctJobs > 1 ? `${text} (strictest of ${String(distinctJobs)} steps)` : text
}

const REFUSAL: Record<StepRefusalReason, string> = {
  outside_ceiling: 'outside the host’s ceiling',
  runner_too_old: 'its runner is too old (before 068)',
  host_revoked: 'the host is revoked',
  no_steps: 'its profiles have no steps',
}

/** "k3s-agent: outside the host's ceiling". */
export function refusalText(job: string, reason: StepRefusalReason): string {
  return reason === 'no_steps' || reason === 'host_revoked' ? REFUSAL[reason] : `${job}: ${REFUSAL[reason]}`
}

// --- The view (converge page) -------------------------------------------------------------------

export interface ViewRow {
  host_id: string
  host: string
  state: ConvergeHostState
  cells: (ConvergeStepView | null)[]
}

export function viewMatrix(view: ConvergeView): Matrix<ViewRow> {
  const positions = columns(view.hosts.map((h) => h.steps.length))
  const rows = view.hosts.map((h) => {
    const at = new Map(h.steps.map((s) => [s.position, s]))
    return { host_id: h.host_id, host: h.host, state: h.state, cells: positions.map((p) => at.get(p) ?? null) }
  })
  return { positions, rows }
}

const STEP_LABEL: Record<StepState, string> = {
  blocked: 'Blocked',
  queued: 'Queued',
  sent: 'Sent',
  running: 'Running',
  succeeded: 'Succeeded',
  failed: 'Failed',
  refused: 'Refused',
  lost: 'Lost',
  cancelled: 'Cancelled',
  skipped: 'Skipped: dependency failed',
  covered: 'Covered',
}

export function stepLabel(state: StepState): string {
  return STEP_LABEL[state]
}

export type Tone = 'ok' | 'err' | 'live' | 'muted'

export function stepTone(state: StepState): Tone {
  switch (state) {
    case 'succeeded':
    case 'covered':
      return 'ok'
    case 'failed':
    case 'refused':
    case 'lost':
      return 'err'
    case 'running':
    case 'sent':
      return 'live'
    default:
      return 'muted'
  }
}

const TONE_CLASS: Record<Tone, string> = {
  ok: 'text-status-ok',
  err: 'text-status-err',
  live: 'text-accent-neon',
  muted: 'text-muted-foreground',
}

/** The text colour for a step's state. */
export function toneClass(state: StepState): string {
  return TONE_CLASS[stepTone(state)]
}

const DONE: readonly StepState[] = ['succeeded', 'covered']

/** Steps Retry failed would run again: every step that did not succeed. */
export function retryableSteps(view: ConvergeView): number {
  return view.hosts.reduce((n, h) => n + h.steps.filter((s) => !DONE.includes(s.state)).length, 0)
}

/** Whether every host's chain has ended. */
export function finished(view: ConvergeView): boolean {
  return view.hosts.every((h) => h.state === 'succeeded' || h.state === 'failed' || h.state === 'cancelled')
}

/** Every run id the view names: the parent and each step's child run. */
export function runIds(view: ConvergeView): Set<string> {
  const ids = new Set([view.run.id])
  for (const h of view.hosts) for (const s of h.steps) ids.add(s.run_id)
  return ids
}

/**
 * The Hosts column: "converging 2/3 · keel-node", "converged", "failed at debian". `steps` (the
 * host's steps from a loaded view) names the job; without it the position stands in.
 */
export function hostConvergeText(c: ConvergeHostSummary, steps?: readonly ConvergeStepView[]): string {
  const job = steps?.find((s) => s.position === c.step)?.job
  const at = `${String(c.step)}/${String(c.of)}`
  switch (c.state) {
    case 'waiting':
      return `waiting · ${String(c.of)} ${c.of === 1 ? 'step' : 'steps'}`
    case 'holding':
      return job ? `converging ${at} · ${job}` : `converging ${at}`
    case 'succeeded':
      return 'converged'
    case 'failed':
      return `failed at ${job ?? `step ${at}`}`
    case 'cancelled':
      return 'converge cancelled'
  }
}

// --- Who may converge ---------------------------------------------------------------------------

/**
 * Whether `me` may converge `host`: Operate on every one of its profiles (an owner or admin, or
 * operator / ws_admin grants in its workspace whose profiles, together, cover the host's). The
 * server decides again.
 */
export function canConverge(me: Me | null, host: HostInfo): boolean {
  if (!me) return false
  if (me.instance_role === 'owner' || me.instance_role === 'admin') return true
  const grants = me.grants.filter((g) => g.workspace === host.workspace && (g.role === 'operator' || g.role === 'ws_admin'))
  if (grants.some((g) => g.profiles.length === 0)) return true
  if (grants.length === 0) return false
  const covered = new Set(grants.flatMap((g) => g.profiles))
  return host.profiles.every((p) => covered.has(p))
}

/** Why Converge… is off for these hosts, or `null` when it is on. */
export function convergeBlocked(me: Me | null, hosts: readonly HostInfo[], binding: string | null): string | null {
  if (binding === null) return 'Bind the workspace’s repository first.'
  if (hosts.length === 0) return 'Select hosts to converge.'
  const out = hosts.filter((h) => !canConverge(me, h))
  if (out.length > 0) {
    const names = out.map((h) => h.inventory_name).join(', ')
    return `You can’t operate on every profile of ${names}.`
  }
  const revoked = hosts.filter((h) => h.state === 'revoked')
  if (revoked.length > 0) return `${revoked.map((h) => h.inventory_name).join(', ')} ${revoked.length === 1 ? 'is' : 'are'} revoked.`
  return null
}
