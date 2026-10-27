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
 */
import type {
  Cmd,
  Event,
  HostStats,
  LogStream,
  RunHostInfo,
  RunInfo,
  RunPhase,
  RunReason,
} from '../../src/protocol/wire'
import type { FakeBackend } from './fake-backend'
import { HEAD, NOW, WORKSPACE } from './fake-repos'

const EXPIRES = '2026-10-27T12:10:00Z'

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
    ...over,
  }
}

type Output =
  | { type: 'RunEvent'; data: Extract<Event, { type: 'RunEvent' }>['data'] }
  | { type: 'RunLog'; data: Extract<Event, { type: 'RunLog' }>['data'] }

export class FakeRuns {
  runs: RunInfo[] = []
  hosts = new Map<string, RunHostInfo[]>()
  /** Stored output per run, in seq order. */
  readonly output = new Map<string, Output[]>()
  /** Every `SubscribeRun` received, in order. */
  readonly subscribes: { run: string; from_seq: number }[] = []
  private readonly pending = new Map<string, string>()
  private seq = 0

  private readonly backend: FakeBackend

  constructor(backend: FakeBackend) {
    this.backend = backend
    backend.on('ListRuns', (cmd) => this.respond(cmd))
    backend.on('GetRun', (cmd) => this.respond(cmd))
    backend.on('SubscribeRun', (cmd) => this.respond(cmd))
    backend.on('UnsubscribeRun', (cmd) => this.respond(cmd))
    backend.on('PreviewCancel', (cmd) => this.respond(cmd))
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
      case 'ListRuns':
        return [
          {
            type: 'Runs',
            data: {
              items: this.runs.filter(
                (r) => r.workspace === cmd.data.workspace && (cmd.data.state === null || r.state === cmd.data.state),
              ),
            },
          },
        ]
      case 'GetRun': {
        const run = this.runs.find((r) => r.id === cmd.data.run)
        if (!run) return [{ type: 'OpError', data: { code: 'not_found', message: 'No such run.' } }]
        return [{ type: 'Run', data: { run, hosts: this.hosts.get(run.id) ?? [] } }]
      }
      case 'SubscribeRun': {
        const { run, from_seq } = cmd.data
        this.subscribes.push({ run, from_seq })
        if (!this.runs.some((r) => r.id === run)) {
          return [{ type: 'OpError', data: { code: 'not_found', message: 'No such run.' } }]
        }
        const after = (this.output.get(run) ?? []).filter((o) => o.data.seq > from_seq)
        return [{ type: 'RunSubscribed', data: { run, seq: Math.max(from_seq, this.lastSeq(run)) } }, ...after]
      }
      case 'UnsubscribeRun':
        return []
      case 'PreviewCancel': {
        const run = this.runs.find((r) => r.id === cmd.data.run)
        if (!run) return [{ type: 'OpError', data: { code: 'not_found', message: 'No such run.' } }]
        const preview_id = `run-preview-${String(++this.seq)}`
        this.pending.set(preview_id, run.id)
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
      case 'Confirm': {
        const run = this.pending.get(cmd.data.preview_id)
        if (run === undefined) return undefined
        this.pending.delete(cmd.data.preview_id)
        this.runs = this.runs.map((r) =>
          r.id === run ? { ...r, state: 'cancelled' as const, reason: 'cancelled_by_user' as const } : r,
        )
        return [
          { type: 'Confirmed', data: { preview_id: cmd.data.preview_id } },
          { type: 'RunState', data: { run, state: 'cancelled', reason: 'cancelled_by_user' } },
        ]
      }
      default:
        return undefined
    }
  }
}
