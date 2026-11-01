/**
 * The fake backend's runs (spec 055 US2, T064).
 *
 * An in-memory model answering the run commands as apps/keel/server/src/runs.rs does, installed on
 * a `FakeBackend` with `on(...)` so the shared fake stays untouched:
 *
 * - `ListRuns` answers `Runs`, newest first; `GetRun` answers `Run` with its hosts.
 * - `SubscribeRun{from_seq}` answers `RunSubscribed` with the last stored seq, then every stored
 *   `RunEvent`/`RunLog` after `from_seq`. Later output is pushed by the spec with `emit`.
 * - `PreviewCancel` previews; its `Confirm` cancels the run and answers `RunState`.
 * - Run approval (spec 064), as apps/keel/server/src/approvals.rs: `PreviewRunApprove` and
 *   `PreviewRunDeny` check the grant (`Denied no_grant`) and the refusals in keel-core's order
 *   (`run_answered`, `approval_expired`, `four_eyes_requester`, `already_answered`), then preview
 *   with `step_up`. `Confirm` checks them again, needs a step-up assertion or a fresh check, and
 *   answers `Confirmed` then `Run`; only a transition (enough approvals, a denial) pushes
 *   `RunState`. A run in `overdue` is past its window: answering it expires it on the spot.
 *   `CountPendingApprovals` counts what the caller could still answer; `ListRuns` with
 *   `pending_approval` answers oldest first.
 */
import type {
  ApprovalEntry,
  Cmd,
  Event,
  HostStats,
  LogStream,
  Me,
  RunApproval,
  RunHostInfo,
  RunInfo,
  RunPhase,
  RunReason,
  WorkspaceCount,
} from '../../src/protocol/wire'
import type { FakeBackend } from './fake-backend'
import { HEAD, NOW, WORKSPACE } from './fake-repos'

const EXPIRES = '2026-10-27T12:10:00Z'

const APPROVE = 'preview_run_approve'
const DENY = 'preview_run_deny'

export function stats(over: Partial<HostStats> = {}): HostStats {
  return { ok: 0, changed: 0, failed: 0, unreachable: 0, skipped: 0, rescued: 0, ignored: 0, ...over }
}

export function runInfo(over: Partial<RunInfo> = {}): RunInfo {
  return {
    id: 'run-0001',
    workspace: WORKSPACE,
    binding: 'binding-0001',
    job: 'job-0001',
    job_name: 'web',
    trigger: 'delivery',
    requested_by: null,
    commit: HEAD,
    inventory: 'prod',
    inventory_revision: 'rev-0001',
    newer_inventory_revision: false,
    limit: null,
    hosts: ['web-1', 'web-2'],
    policy: null,
    state: 'running',
    reason: null,
    created_at: NOW,
    started_at: NOW,
    ended_at: null,
    summary: null,
    params: {},
    approval: null,
    ...over,
  }
}

/** A run held for approval (spec 064): one approval needed, no four-eyes, a day's window. */
export function hold(over: Partial<RunInfo> = {}, approval: Partial<RunApproval> = {}): RunInfo {
  return runInfo({
    state: 'pending_approval',
    started_at: null,
    approval: { needed: 1, four_eyes: false, expires_at: '2026-10-28T12:00:00Z', approvals: [], denial: null, ...approval },
    ...over,
  })
}

/** One recorded answer, someone else's unless `over` says otherwise. */
export function approvalEntry(over: Partial<ApprovalEntry> = {}): ApprovalEntry {
  return { user: '01J0000000000000000000BOB', name: 'Bob Builder', role: 'approver', at: NOW, reason: null, ...over }
}

/**
 * The role an answer is recorded under, or `null` without Approve on the whole workspace: the
 * server's `effective_role`.
 */
export function approverRole(me: Me | null, workspace: string): string | null {
  if (!me) return null
  if (me.instance_role === 'owner' || me.instance_role === 'admin') return me.instance_role
  const roles = me.grants.filter((g) => g.workspace === workspace && g.profiles.length === 0).map((g) => g.role)
  if (roles.includes('ws_admin')) return 'ws_admin'
  return roles.includes('approver') ? 'approver' : null
}

type Output =
  | { type: 'RunEvent'; data: Extract<Event, { type: 'RunEvent' }>['data'] }
  | { type: 'RunLog'; data: Extract<Event, { type: 'RunLog' }>['data'] }

type Answer = { kind: 'approve' | 'deny'; run: string; reason: string | null }
type Pending = { kind: 'cancel'; run: string; reason: null } | Answer

function opError(code: string, message: string): Event {
  return { type: 'OpError', data: { code, message } }
}

export class FakeRuns {
  runs: RunInfo[] = []
  hosts = new Map<string, RunHostInfo[]>()
  /** Stored output per run, in seq order. */
  readonly output = new Map<string, Output[]>()
  /** Every `SubscribeRun` received, in order. */
  readonly subscribes: { run: string; from_seq: number }[] = []
  /** Held runs past their approval window, which the sweeper has not reached yet. */
  readonly overdue = new Set<string>()
  private readonly pending = new Map<string, Pending>()
  private seq = 0

  private readonly backend: FakeBackend

  constructor(backend: FakeBackend) {
    this.backend = backend
    backend.on('ListRuns', (cmd) => this.respond(cmd))
    backend.on('GetRun', (cmd) => this.respond(cmd))
    backend.on('SubscribeRun', (cmd) => this.respond(cmd))
    backend.on('UnsubscribeRun', (cmd) => this.respond(cmd))
    backend.on('PreviewCancel', (cmd) => this.respond(cmd))
    backend.on('PreviewRunApprove', (cmd) => this.respond(cmd))
    backend.on('PreviewRunDeny', (cmd) => this.respond(cmd))
    backend.on('CountPendingApprovals', (cmd) => this.respond(cmd))
    // Falls through (undefined) for previews that are not a run's.
    backend.on('Confirm', (cmd) => this.respond(cmd))
  }

  add(run: RunInfo, hosts: RunHostInfo[] = []): void {
    this.runs = [run, ...this.runs.filter((r) => r.id !== run.id)]
    this.hosts.set(run.id, hosts)
  }

  private lastSeq(run: string): number {
    return this.output.get(run)?.at(-1)?.data.seq ?? 0
  }

  /** Stores the next output line for `run` and pushes it live. */
  log(run: string, text: string, stream: LogStream = 'stdout'): void {
    this.emit({ type: 'RunLog', data: { run, seq: this.lastSeq(run) + 1, stream, text } })
  }

  /** Stores the next event for `run` and pushes it live. */
  event(run: string, kind: string, host: string | null, task: string | null): void {
    this.emit({
      type: 'RunEvent',
      data: { run, seq: this.lastSeq(run) + 1, at: NOW, kind, host, task, data: '{}' },
    })
  }

  /** Stores output without pushing it: what a socket that was away missed. */
  store(o: Output): void {
    this.output.set(o.data.run, [...(this.output.get(o.data.run) ?? []), o])
  }

  private emit(o: Output): void {
    this.store(o)
    this.backend.push(o)
  }

  /** Moves a run to `state`, with its hosts as they ended, and pushes `RunState`. */
  finish(run: string, state: RunPhase, reason: RunReason | null, hosts?: RunHostInfo[]): void {
    this.runs = this.runs.map((r) => (r.id === run ? { ...r, state, reason, ended_at: NOW } : r))
    if (hosts) this.hosts.set(run, hosts)
    this.backend.push({ type: 'RunState', data: { run, state, reason } })
  }

  private respond(cmd: Cmd): Event[] | undefined {
    switch (cmd.type) {
      case 'ListRuns': {
        const items = this.runs.filter(
          (r) => r.workspace === cmd.data.workspace && (cmd.data.state === null || r.state === cmd.data.state),
        )
        // What has waited longest comes first.
        if (cmd.data.state === 'pending_approval') items.sort((a, b) => a.created_at.localeCompare(b.created_at))
        return [{ type: 'Runs', data: { items } }]
      }
      case 'GetRun': {
        const run = this.runs.find((r) => r.id === cmd.data.run)
        if (!run) return [opError('not_found', 'No such run.')]
        return [{ type: 'Run', data: { run, hosts: this.hosts.get(run.id) ?? [] } }]
      }
      case 'SubscribeRun': {
        const { run, from_seq } = cmd.data
        this.subscribes.push({ run, from_seq })
        if (!this.runs.some((r) => r.id === run)) return [opError('not_found', 'No such run.')]
        const after = (this.output.get(run) ?? []).filter((o) => o.data.seq > from_seq)
        return [{ type: 'RunSubscribed', data: { run, seq: Math.max(from_seq, this.lastSeq(run)) } }, ...after]
      }
      case 'UnsubscribeRun':
        return []
      case 'PreviewCancel': {
        const run = this.runs.find((r) => r.id === cmd.data.run)
        if (!run) return [opError('not_found', 'No such run.')]
        const preview_id = `run-preview-${String(++this.seq)}`
        this.pending.set(preview_id, { kind: 'cancel', run: run.id, reason: null })
        return [
          {
            type: 'Preview',
            data: {
              preview_id,
              action: 'preview_cancel',
              summary: `Cancel run ${run.id} of job ${run.job_name}.`,
              affected: run.hosts,
              step_up: false,
              expires_at: EXPIRES,
            },
          },
        ]
      }
      case 'PreviewRunApprove':
        return this.previewAnswer('approve', cmd.data.run, null)
      case 'PreviewRunDeny':
        return this.previewAnswer('deny', cmd.data.run, cmd.data.reason)
      case 'CountPendingApprovals':
        return [{ type: 'PendingApprovals', data: { counts: this.counts() } }]
      case 'Confirm': {
        const { preview_id } = cmd.data
        const p = this.pending.get(preview_id)
        if (p === undefined) return undefined
        this.pending.delete(preview_id)
        if (p.kind !== 'cancel') return this.confirmAnswer(p, preview_id, cmd.data.step_up !== undefined)
        this.runs = this.runs.map((r) =>
          r.id === p.run ? { ...r, state: 'cancelled' as const, reason: 'cancelled_by_user' as const } : r,
        )
        return [
          { type: 'Confirmed', data: { preview_id } },
          { type: 'RunState', data: { run: p.run, state: 'cancelled', reason: 'cancelled_by_user' } },
        ]
      }
      default:
        return undefined
    }
  }

  // ---- approval (spec 064) --------------------------------------------------------------------

  /** The first refusal in keel-core's order, or `null` when `kind` may go ahead. */
  private refusal(kind: 'approve' | 'deny', run: RunInfo): Event | null {
    const me = this.backend.me
    if (run.state !== 'pending_approval') {
      return opError('run_answered', `This run has already been answered; it is now ${run.state}.`)
    }
    if (this.overdue.has(run.id)) {
      // The server expires it on the spot rather than wait for the sweeper.
      this.overdue.delete(run.id)
      this.finish(run.id, 'expired', 'approval_expired')
      return opError('approval_expired', 'This run’s approval window has closed.')
    }
    if (kind === 'approve' && run.approval?.four_eyes && run.requested_by === me?.id) {
      return opError('four_eyes_requester', 'You started this run, so someone else must approve it.')
    }
    if (run.approval?.approvals.some((a) => a.user === me?.id)) {
      return opError('already_answered', 'You have already answered this run.')
    }
    return null
  }

  private previewAnswer(kind: 'approve' | 'deny', id: string, reason: string | null): Event[] {
    const run = this.runs.find((r) => r.id === id)
    if (!run) return [opError('not_found', 'No such run.')]
    const action = kind === 'approve' ? APPROVE : DENY
    if (approverRole(this.backend.me, run.workspace) === null) {
      return [{ type: 'Denied', data: { action, reason: 'no_grant' } }]
    }
    const refused = this.refusal(kind, run)
    if (refused) return [refused]
    const preview_id = `run-preview-${String(++this.seq)}`
    this.pending.set(preview_id, { kind, run: id, reason })
    const k = (run.approval?.approvals.length ?? 0) + 1
    const needed = run.approval?.needed ?? 1
    const effect =
      kind === 'deny'
        ? 'it will not run'
        : k >= needed
          ? 'this queues the run'
          : `this adds approval ${String(k)} of ${String(needed)}`
    return [
      {
        type: 'Preview',
        data: {
          preview_id,
          action,
          summary:
            `${kind === 'approve' ? 'Approve' : 'Deny'} run of job ${run.job_name} at ${run.commit.slice(0, 12)} ` +
            `on ${String(run.hosts.length)} host(s) from inventory ${run.inventory}: ${effect}.`,
          affected: run.hosts,
          step_up: true,
          expires_at: EXPIRES,
        },
      },
    ]
  }

  private confirmAnswer(p: Answer, preview_id: string, steppedUp: boolean): Event[] {
    const me = this.backend.me
    if (!me) return []
    if (!steppedUp && !me.verification.verified) {
      return [opError('step_up_required', 'Confirm it’s you with a passkey first.')]
    }
    const confirmed: Event = { type: 'Confirmed', data: { preview_id } }
    const run = this.runs.find((r) => r.id === p.run)
    if (!run?.approval) return [confirmed, opError('not_found', 'No such run.')]
    const role = approverRole(me, run.workspace)
    if (role === null) {
      return [confirmed, { type: 'Denied', data: { action: p.kind === 'approve' ? APPROVE : DENY, reason: 'no_grant' } }]
    }
    const refused = this.refusal(p.kind, run)
    if (refused) return [confirmed, refused]
    const entry: ApprovalEntry = { user: me.id, name: me.display_name, role, at: NOW, reason: p.reason }
    let next: RunInfo
    if (p.kind === 'deny') {
      next = { ...run, state: 'denied', reason: 'approval_denied', ended_at: NOW, approval: { ...run.approval, denial: entry } }
    } else {
      const approvals = [...run.approval.approvals, entry]
      next = { ...run, approval: { ...run.approval, approvals } }
      if (approvals.length >= run.approval.needed) next = { ...next, state: 'queued' }
    }
    this.runs = this.runs.map((r) => (r.id === run.id ? next : r))
    // Only a transition pushes `RunState`; another approval short of `needed` does not.
    if (next.state !== run.state) {
      this.backend.push({ type: 'RunState', data: { run: run.id, state: next.state, reason: next.reason } })
    }
    return [confirmed, { type: 'Run', data: { run: next, hosts: this.hosts.get(run.id) ?? [] } }]
  }

  /**
   * What the caller could still answer, per workspace with any: not runs they answered, not
   * four-eyes runs they started, not runs past their window.
   */
  private counts(): WorkspaceCount[] {
    const me = this.backend.me
    const counts = new Map<string, number>()
    for (const r of this.runs) {
      if (r.state !== 'pending_approval' || !r.approval || this.overdue.has(r.id)) continue
      if (approverRole(me, r.workspace) === null) continue
      if (r.approval.approvals.some((a) => a.user === me?.id)) continue
      if (r.approval.four_eyes && r.requested_by === me?.id) continue
      counts.set(r.workspace, (counts.get(r.workspace) ?? 0) + 1)
    }
    return [...counts].map(([workspace, count]) => ({ workspace, count }))
  }
}
