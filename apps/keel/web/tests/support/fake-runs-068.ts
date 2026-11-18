/**
 * Spec 068's run and binding commands for the fake backend (T044, T051, T056, T060, T066), kept
 * apart from `fake-runs.ts` and `fake-repos.ts`:
 *
 * - `ListJobs` answers the catalog's jobs with the params and credentials set here (`job()`).
 * - `PreviewRunStart` checks `params` like the server (`param_invalid`) and previews; its Confirm
 *   adds the run to `FakeRuns`.
 * - `PreviewRequiredChecks` validates like `server/src/checks.rs` (or refuses with `refuse`), previews
 *   with step-up, and its Confirm updates the binding and sends `Binding`.
 * - `checks(run, ...)` pushes `RunChecksChanged`.
 *
 * Construct it after `FakeRuns`: it chains `Confirm` in front of every earlier handler.
 */
import type { BindingInfo, Event, JobInfo, ParamView, RunCheckView } from '../../src/protocol/wire'
import type { FakeBackend } from './fake-backend'
import { fixtureJobs } from './catalog-fixtures'
import { runInfo, type FakeRuns } from './fake-runs'

const EXPIRES = '2026-10-28T12:10:00Z'
const DIGEST = /^sha256:[0-9a-f]{64}$/

function opError(code: string, message: string): Event {
  return { type: 'OpError', data: { code, message } }
}

export function choiceParam(allowed: string[] | null, over: Partial<ParamView> = {}): ParamView {
  return { kind: 'choice', allowed, default: allowed?.[0] ?? null, unavailable: null, ...over }
}

export class FakeRuns068 {
  /** Per job name: what to add to the catalog's fixture job. */
  readonly jobs = new Map<string, Partial<JobInfo>>()
  /** Every `PreviewRunStart` received. */
  readonly started: { job: string; params: Record<string, string>; check: boolean }[] = []
  /** A refusal `PreviewRequiredChecks` answers instead of validating, e.g. `checks_unsupported`. */
  refuse: { code: string; message: string } | null = null
  private readonly previews = new Map<string, () => Event[]>()
  private n = 0
  private readonly backend: FakeBackend
  private readonly runs: FakeRuns

  constructor(backend: FakeBackend, runs: FakeRuns) {
    this.backend = backend
    this.runs = runs
    backend.chain('ListJobs', (cmd) => {
      if (this.jobs.size === 0) return undefined
      const items = fixtureJobs(cmd.data.binding).map((j) => ({ ...j, ...this.jobs.get(j.name) }))
      return [{ type: 'Jobs', data: { binding: cmd.data.binding, items } }]
    })
    backend.on('PreviewRunStart', (cmd) => this.runStart(cmd.data))
    backend.on('PreviewRequiredChecks', (cmd) => this.requiredChecks(cmd.data))
    backend.chain('Confirm', (cmd) => {
      const apply = this.previews.get(cmd.data.preview_id)
      if (!apply) return undefined
      this.previews.delete(cmd.data.preview_id)
      return [{ type: 'Confirmed', data: { preview_id: cmd.data.preview_id } }, ...apply()]
    })
  }

  /** Adds params, credentials or anything else to the fixture job `name`. */
  job(name: string, over: Partial<JobInfo>): this {
    this.jobs.set(name, { ...this.jobs.get(name), ...over })
    return this
  }

  /** Pushes the run's required checks as they moved. */
  checks(run: string, checks: RunCheckView[]): void {
    const r = this.runs.runs.find((x) => x.id === run)
    if (r) r.checks = checks
    this.backend.push({ type: 'RunChecksChanged', data: { run_id: run, checks } })
  }

  private preview(action: string, summary: string, step_up: boolean, apply: () => Event[]): Event[] {
    const preview_id = `preview-068-${String(++this.n)}`
    this.previews.set(preview_id, apply)
    return [{ type: 'Preview', data: { preview_id, action, summary, affected: [], step_up, expires_at: EXPIRES } }]
  }

  private runStart(data: { job: string; params: Record<string, string>; check: boolean }): Event[] {
    this.started.push({ job: data.job, params: { ...data.params }, check: data.check })
    const name = data.job.replace(/^job-/, '')
    const job = { ...fixtureJobs('binding-ops').find((j) => j.name === name), ...this.jobs.get(name) }
    const declared = job.params ?? {}
    for (const [p, v] of Object.entries(data.params)) {
      const view = declared[p]
      if (!view) return [opError('param_invalid', `${name} declares no param ${p}.`)]
      const ok = view.kind === 'choice' ? view.unavailable === null && (view.allowed ?? []).includes(v) : DIGEST.test(v)
      if (!ok) return [opError('param_invalid', `${v} is not an allowed value of ${p}.`)]
    }
    const lines = [
      ...(data.check ? ['Check (ansible-playbook --check --diff, changes nothing).'] : []),
      `Run ${name} at the binding's head.`,
      ...Object.entries(data.params).map(([p, v]) => `${p} = ${v}`),
      ...((job.credentials ?? []).length > 0 ? [`Credentials sent: ${(job.credentials ?? []).join(', ')}.`] : []),
    ]
    return this.preview('run.start', lines.join(' '), false, () => {
      const run = runInfo({
        id: `run-start-${String(this.n)}`,
        job: data.job,
        job_name: name,
        trigger: 'manual',
        state: 'queued',
        check: data.check,
        params: data.params,
        started_at: null,
      })
      this.runs.add(run)
      return [{ type: 'Run', data: { run, hosts: [] } }]
    })
  }

  private binding(id: string): BindingInfo | undefined {
    return this.backend.repos.bindings.find((b) => b.id === id)
  }

  private requiredChecks(data: { binding_id: string; contexts: string[]; timeout_secs?: number }): Event[] {
    const b = this.binding(data.binding_id)
    if (!b) return [opError('not_found', 'No such binding.')]
    if (this.refuse) return [opError(this.refuse.code, this.refuse.message)]
    const set = [...new Set(data.contexts.map((c) => c.trim()))].sort()
    if (set.some((c) => c === '' || c.length > 200)) return [opError('invalid_context', 'A check context is 1 to 200 printable characters.')]
    if (set.length > 16) return [opError('too_many_contexts', 'A binding can require at most 16 checks.')]
    const t = data.timeout_secs
    if (t !== undefined && (t < 60 || t > 86400)) return [opError('invalid_timeout', 'The check timeout is 60 to 86400 seconds.')]
    const summary =
      set.length === 0
        ? `Require no checks for ${b.url} (${b.branch}): runs no longer wait on the forge.`
        : `Require the checks ${set.join(', ')} to pass on ${b.url} (${b.branch}) before a run from a commit moves on; runs wait up to ${String(t ?? b.checks_timeout_secs)} seconds.`
    return this.preview('binding.required_checks_set', summary, true, () => {
      b.required_checks = set.map((context) => ({ context }))
      if (t !== undefined) b.checks_timeout_secs = t
      // As the server: the binding again, with its jobs, as `GetBinding` would answer.
      return this.backend.repos.respond({ type: 'GetBinding', data: { binding: b.id } }) ?? []
    })
  }
}
